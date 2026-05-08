# Complete Solidity Specification for HDC Smart Contracts

> **Status: PARTIALLY SUPERSEDED by PR #42.**
> - `PheromoneRegistry.sol` is **superseded** by the StigmergyPrecompile (Rust, address 0xA0D) in PR #42.
> - `InsightBoard.sol` is **still active** but lives in the `contracts-core` repo; its precompile calls must be updated.
> - `HdcLib.sol` has **alignment issues**: Solidity opcode assignments do not match the Rust precompile (see Section 9 below).
> - The precompile address has moved from `0x09` to `0xA0C` in PR #42.
>
> **Key decision required:** Reconcile the Solidity contract surface with PR #42's
> Rust precompile surface. See the reconciliation checklist at the end of this document.

**Version:** 1.0 (with PR #42 annotations added 2026-05-08)
**Target:** Solidity ^0.8.20, deployed on daeji chain
**Source documents:** 07-shared-substrate.md, 09-optimal-design.md
**Purpose:** An implementing agent uses this document to produce the final .sol files.

---

## Table of Contents

1. [Contract Inventory](#contract-inventory)
2. [HdcPrecompile.sol (Library Interface)](#1-hdcprecompilesol)
3. [IInsightBoard.sol (Interface)](#2-iinsightboardsol)
4. [IPheromoneRegistry.sol (Interface)](#3-ipheromoneregistrysol)
5. [InsightBoard.sol (Implementation)](#4-insightboardsol)
6. [PheromoneRegistry.sol (Implementation)](#5-pheromoneregistrysol)
7. [Deployment Script](#6-deployment-script)
8. [Test Scenarios (Foundry)](#7-test-scenarios)
9. [Anti-Pattern Checklist](#8-anti-pattern-checklist)

---

## Contract Inventory

| File | Type | Purpose |
|------|------|---------|
| `HdcPrecompile.sol` | Library | Typed wrappers for calling the 0x09 precompile |
| `IInsightBoard.sol` | Interface | External-facing API for InsightBoard consumers |
| `IPheromoneRegistry.sol` | Interface | External-facing API for PheromoneRegistry consumers |
| `InsightBoard.sol` | Contract | 7-state FSM for on-chain knowledge lifecycle |
| `PheromoneRegistry.sol` | Contract | Stigmergic pheromone deposit/decay/SINR |

---

## 1. HdcPrecompile.sol

### Purpose

Typed Solidity library for calling the HDC precompile deployed at address
`0x09`. Current Rust code exposes a stateless raw-opcode precompile; it does
not maintain a mutable in-memory EVM index. Node-local indexes should be rebuilt
from finalized events unless a future hardfork explicitly adds consensus-backed
stateful precompile opcodes. This library must therefore share one opcode table,
payload schema, and output schema with `crates/hdc/chain/src/precompile.rs`.

### Complete Solidity Code

```solidity
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.20;

/// @title HdcLib
/// @notice Library for calling the HDC precompile at address 0x09.
/// @dev The v1 precompile uses a 1-byte raw-opcode dispatch:
///      0x01 = hamming, 0x02 = bind, 0x03 = bundle,
///      0x04 = permute, 0x05 = vectorId, 0x06 = isSimilar.
///      Stateful store/search/delete opcodes are not part of v1.
///      All vectors are 1,280 bytes (10,240 bits / 8).
library HdcLib {
    address internal constant HDC_PRECOMPILE = address(0x09);

    /// @notice Compute the Hamming distance between two raw vectors.
    /// @param a First 1,280-byte vector.
    /// @param b Second 1,280-byte vector.
    /// @return dist The number of differing bits (0..10240).
    function hamming(bytes memory a, bytes memory b) internal view returns (uint32 dist) {
        require(a.length == 1280 && b.length == 1280, "HdcLib: invalid vector size");
        bytes memory payload = abi.encodePacked(uint8(0x01), a, b);
        (bool ok, bytes memory ret) = HDC_PRECOMPILE.staticcall(payload);
        require(ok && ret.length == 32, "HdcLib: hamming failed");
        dist = abi.decode(ret, (uint32));
    }

    /// @notice XOR-bind two vectors. Result is quasi-orthogonal to both inputs.
    /// @param a First 1,280-byte vector.
    /// @param b Second 1,280-byte vector.
    /// @return result The XOR of a and b (1,280 bytes).
    function bind(bytes memory a, bytes memory b) internal view returns (bytes memory result) {
        require(a.length == 1280 && b.length == 1280, "HdcLib: invalid vector size");
        bytes memory payload = abi.encodePacked(uint8(0x02), a, b);
        (bool ok, bytes memory ret) = HDC_PRECOMPILE.staticcall(payload);
        require(ok && ret.length == 1280, "HdcLib: bind failed");
        result = ret;
    }

    /// @notice Majority-vote bundle of N vectors.
    /// @param vectors Array of 1,280-byte vectors.
    /// @return result The bundled vector (1,280 bytes).
    function bundle(bytes[] memory vectors) internal view returns (bytes memory result) {
        require(vectors.length > 0, "HdcLib: empty bundle");
        // Encode: opcode(0x03) + count(uint32 big-endian) + concatenated vectors.
        bytes memory payload = abi.encodePacked(uint8(0x03), uint32(vectors.length));
        for (uint256 i = 0; i < vectors.length; i++) {
            require(vectors[i].length == 1280, "HdcLib: invalid vector size");
            payload = abi.encodePacked(payload, vectors[i]);
        }
        (bool ok, bytes memory ret) = HDC_PRECOMPILE.staticcall(payload);
        require(ok && ret.length == 1280, "HdcLib: bundle failed");
        result = ret;
    }

    /// @notice Cyclic left-rotation of a vector by n bit positions.
    /// @param v The 1,280-byte vector to permute.
    /// @param n Number of bit positions to rotate left.
    /// @return result The permuted vector (1,280 bytes).
    function permute(bytes memory v, uint32 n) internal view returns (bytes memory result) {
        require(v.length == 1280, "HdcLib: invalid vector size");
        bytes memory payload = abi.encodePacked(uint8(0x04), v, n);
        (bool ok, bytes memory ret) = HDC_PRECOMPILE.staticcall(payload);
        require(ok && ret.length == 1280, "HdcLib: permute failed");
        result = ret;
    }

    /// @notice Store a vector in the precompile's in-memory index.
    /// @dev NOT AVAILABLE in the current Rust v1 precompile. Keep this out of
    ///      production contract paths unless stateful opcodes are added.
    /// @dev This is a state-changing call (not staticcall). Emits an event
    ///      inside the precompile for index reconstruction on node restart.
    /// @param id The bytes32 key for the vector.
    /// @param vector The 1,280-byte vector.
    function storeVector(bytes32 id, bytes memory vector) internal {
        id;
        vector;
        revert("HdcLib: storeVector unavailable in v1");
    }

    /// @notice Search the precompile index for the topK nearest vectors.
    /// @dev NOT AVAILABLE in the current Rust v1 precompile. Use finalized
    ///      events plus node-local indexing, or add a consensus-backed v2 opcode.
    /// @param query The 1,280-byte query vector.
    /// @param topK Number of results to return (max 255).
    /// @return ids The bytes32 keys of the nearest vectors.
    /// @return distances The Hamming distances (uint16) to the query.
    function searchSimilar(
        bytes memory query,
        uint8 topK
    ) internal view returns (bytes32[] memory ids, uint16[] memory distances) {
        query;
        topK;
        revert("HdcLib: searchSimilar unavailable in v1");
    }

    /// @notice Remove a vector from the precompile index.
    /// @dev NOT AVAILABLE in the current Rust v1 precompile.
    /// @param id The bytes32 key of the vector to remove.
    function deleteVector(bytes32 id) internal {
        id;
        revert("HdcLib: deleteVector unavailable in v1");
    }
}
```

### Storage Layout

HdcLib is a library with no storage. Under the current Rust v1 design, no
contract-visible vector index lives inside the precompile. Durable discovery
state must be reconstructed from emitted events by node-local indexers, or a
future stateful precompile design must include consensus-state semantics.

### Gas Analysis

| Function | Gas Cost | Notes |
|----------|----------|-------|
| `hamming` | TBD after benchmark | 160-word XOR + popcount; current Rust gas is known underpriced |
| `bind` | TBD after benchmark | XOR of 160 words |
| `bundle` | TBD after benchmark | Base cost + linear per input; must cap vector count |
| `permute` | ~3,000 | Single vector + rotation |
| `storeVector` | N/A in v1 | Not implemented by Rust precompile |
| `searchSimilar` | N/A in v1 | Not implemented by Rust precompile |
| `deleteVector` | N/A in v1 | Not implemented by Rust precompile |

### Security Checklist

- No reentrancy risk: library functions, no callbacks.
- Integer overflow: Solidity 0.8+ built-in checks.
- Access control: library functions inherit caller's context.
- Front-running: N/A (read-only for hamming/bind/bundle/permute).

### ABI

| Function | Inputs | Outputs |
|----------|--------|---------|
| `hamming(bytes,bytes)` | `(bytes a, bytes b)` | `(uint32 dist)` |
| `bind(bytes,bytes)` | `(bytes a, bytes b)` | `(bytes result)` |
| `bundle(bytes[])` | `(bytes[] vectors)` | `(bytes result)` |
| `permute(bytes,uint32)` | `(bytes v, uint32 n)` | `(bytes result)` |
| `storeVector(bytes32,bytes)` | `(bytes32 id, bytes vector)` | none |
| `searchSimilar(bytes,uint8)` | `(bytes query, uint8 topK)` | `(bytes32[] ids, uint16[] distances)` |
| `deleteVector(bytes32)` | `(bytes32 id)` | none |

---

## 2. IInsightBoard.sol

### Purpose

Clean interface for external consumers of the InsightBoard contract. Agents,
frontends, and other contracts code against this interface.

### Complete Solidity Code

```solidity
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.20;

/// @title IInsightBoard
/// @notice Interface for the InsightBoard -- 7-state FSM for on-chain
///         knowledge lifecycle on the daeji shared substrate.
interface IInsightBoard {
    // ---------------------------------------------------------------
    // Enums
    // ---------------------------------------------------------------

    /// @notice The kind of knowledge entry.
    enum Kind {
        INSIGHT,          // 0 -- general observation
        HEURISTIC,        // 1 -- rule of thumb
        ANTI_KNOWLEDGE,   // 2 -- contradiction / disproof
        WARNING,          // 3 -- urgent hazard signal
        CAUSAL_LINK,      // 4 -- cause-effect relationship
        STRATEGY          // 5 -- actionable plan fragment
    }

    /// @notice Lifecycle state of an insight.
    enum State {
        SUBMITTED,   // 0 -- published, awaiting first confirmation
        VERIFIED,    // 1 -- placeholder for future verification pipeline
        ACTIVE,      // 2 -- confirmed and participating in search
        CHALLENGED,  // 3 -- anti-knowledge published against this entry
        DECAYING,    // 4 -- age > 1x effective half-life, no recent confirmation
        ARCHIVED,    // 5 -- age > 5x effective half-life
        PURGED       // 6 -- age > 10x effective half-life, eligible for removal
    }

    /// @notice Memory tier controlling effective half-life.
    enum Tier {
        TRANSIENT,     // 0 -- multiplier 1x
        WORKING,       // 1 -- multiplier 3x
        CONSOLIDATED,  // 2 -- multiplier 7x
        PERSISTENT     // 3 -- multiplier 10x
    }

    // ---------------------------------------------------------------
    // Events
    // ---------------------------------------------------------------

    /// @notice Emitted when a new insight is submitted.
    /// @param insightId Unique identifier (keccak256 of vectorHash + author + block).
    /// @param vectorHash keccak256 of the full 1,280-byte vector.
    /// @param author Address of the submitting agent.
    /// @param vector Full 1,280-byte HDC vector (stored in event log only).
    /// @param content Arbitrary content payload (stored in event log only).
    /// @param kind The Kind enum value.
    /// @param tier Initial tier (always TRANSIENT on submission).
    event InsightPublished(
        bytes32 indexed insightId,
        bytes32 indexed vectorHash,
        address indexed author,
        bytes vector,
        bytes content,
        uint8 kind,
        uint8 tier
    );

    /// @notice Emitted when an insight is confirmed by a new agent.
    event InsightConfirmed(
        bytes32 indexed insightId,
        address indexed confirmer,
        uint64 totalConfirmations
    );

    /// @notice Emitted when an insight is challenged via anti-knowledge.
    event InsightChallenged(
        bytes32 indexed insightId,
        bytes32 indexed challengingInsightId,
        address indexed challenger
    );

    /// @notice Emitted when an insight's state changes (any transition).
    event InsightStateChanged(
        bytes32 indexed insightId,
        uint8 oldState,
        uint8 newState
    );

    /// @notice Emitted when an ARCHIVED insight is renewed.
    event InsightRenewed(bytes32 indexed insightId, address indexed renewer);

    /// @notice Emitted when a PURGED insight is removed from storage.
    event InsightPurged(bytes32 indexed insightId, address indexed purger);

    // ---------------------------------------------------------------
    // Write functions
    // ---------------------------------------------------------------

    /// @notice Submit a new insight with stake.
    /// @param kind The knowledge kind.
    /// @param vector Full 1,280-byte HDC vector.
    /// @param content Arbitrary content bytes (metadata, text, etc.).
    /// @return insightId The unique identifier for this insight.
    function submit(
        Kind kind,
        bytes calldata vector,
        bytes calldata content
    ) external payable returns (bytes32 insightId);

    /// @notice Confirm an existing insight. Caller must not have already confirmed.
    ///         Accepts insights in SUBMITTED, ACTIVE, DECAYING, or CHALLENGED state.
    /// @param insightId The insight to confirm.
    function confirm(bytes32 insightId) external;

    /// @notice Challenge an ACTIVE insight by referencing anti-knowledge.
    /// @param targetInsightId The ACTIVE insight being challenged.
    /// @param antiKnowledgeId The ANTI_KNOWLEDGE insight that contradicts it.
    function challenge(bytes32 targetInsightId, bytes32 antiKnowledgeId) external;

    /// @notice Renew an ARCHIVED insight. Requires fresh stake >= MIN_STAKE.
    /// @param insightId The ARCHIVED insight to renew.
    function renew(bytes32 insightId) external payable;

    /// @notice Purge a PURGED insight. Clears storage, removes from precompile
    ///         index, returns 10% of stake to original author.
    /// @param insightId The PURGED insight to remove.
    function purge(bytes32 insightId) external;

    // ---------------------------------------------------------------
    // View functions
    // ---------------------------------------------------------------

    /// @notice Compute the current lifecycle state of an insight based on
    ///         age relative to lastConfirmedBlock, tier, and kind.
    /// @param insightId The insight to query.
    /// @return The current State.
    function computeState(bytes32 insightId) external view returns (State);

    /// @notice Search for similar vectors via the HDC precompile.
    /// @param queryVector 1,280-byte query vector.
    /// @param topK Number of results (max 255).
    /// @return ids Matching insight IDs.
    /// @return distances Hamming distances.
    function searchSimilar(
        bytes calldata queryVector,
        uint8 topK
    ) external view returns (bytes32[] memory ids, uint16[] memory distances);
}
```

### ABI

| Function | Selector | Inputs | Outputs |
|----------|----------|--------|---------|
| `submit` | `submit(uint8,bytes,bytes)` | `(uint8 kind, bytes vector, bytes content)` | `(bytes32 insightId)` |
| `confirm` | `confirm(bytes32)` | `(bytes32 insightId)` | none |
| `challenge` | `challenge(bytes32,bytes32)` | `(bytes32 targetInsightId, bytes32 antiKnowledgeId)` | none |
| `renew` | `renew(bytes32)` | `(bytes32 insightId)` | none |
| `purge` | `purge(bytes32)` | `(bytes32 insightId)` | none |
| `computeState` | `computeState(bytes32)` | `(bytes32 insightId)` | `(uint8 state)` |
| `searchSimilar` | `searchSimilar(bytes,uint8)` | `(bytes queryVector, uint8 topK)` | `(bytes32[], uint16[])` |

---

## 3. IPheromoneRegistry.sol

### Purpose

Clean interface for external consumers of the PheromoneRegistry.

### Complete Solidity Code

```solidity
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.20;

/// @title IPheromoneRegistry
/// @notice Interface for the PheromoneRegistry -- stigmergic coordination
///         through digital pheromones with exponential decay and SINR
///         interference modeling.
interface IPheromoneRegistry {
    // ---------------------------------------------------------------
    // Enums
    // ---------------------------------------------------------------

    /// @notice Pheromone signal types with distinct half-lives.
    enum PheromoneType {
        THREAT,       // 0 -- half-life 100 blocks (~40s)
        OPPORTUNITY,  // 1 -- half-life 250 blocks (~100s)
        WISDOM        // 2 -- half-life 1000 blocks (~400s)
    }

    // ---------------------------------------------------------------
    // Events
    // ---------------------------------------------------------------

    /// @notice Emitted when a new pheromone is deposited.
    event PheromoneDeposited(
        bytes32 indexed pheromoneId,
        bytes32 indexed locationHash,
        address indexed depositor,
        uint8 pType,
        uint64 intensity,
        uint64 depositBlock
    );

    /// @notice Emitted when a pheromone is confirmed by another agent.
    ///         Confirmation reduces the half-life (alpha paradox).
    event PheromoneConfirmed(
        bytes32 indexed pheromoneId,
        address indexed confirmer,
        uint64 newConfirmationCount,
        uint64 newEffectiveHalfLife
    );

    /// @notice Emitted when a dead pheromone is pruned from storage.
    event PheromonePruned(
        bytes32 indexed pheromoneId,
        address indexed pruner
    );

    // ---------------------------------------------------------------
    // Write functions
    // ---------------------------------------------------------------

    /// @notice Deposit a pheromone at a location in HDC space.
    /// @param location 1,280-byte HDC vector identifying the region.
    /// @param pType The pheromone type (THREAT, OPPORTUNITY, or WISDOM).
    /// @param intensity Initial signal strength (100..10000).
    function deposit(
        bytes calldata location,
        PheromoneType pType,
        uint64 intensity
    ) external payable;

    /// @notice Confirm an existing pheromone. Reduces its half-life
    ///         per the alpha paradox: new_hl = base_hl / (1 + n_confs).
    /// @param pheromoneId The pheromone to confirm.
    function confirm(bytes32 pheromoneId) external;

    /// @notice Prune a dead pheromone (intensity < death threshold).
    ///         Clears storage slots. Caller receives SSTORE gas refund.
    /// @param pheromoneId The dead pheromone to remove.
    function cleanup(bytes32 pheromoneId) external;

    /// @notice Batch-prune multiple dead pheromones in one transaction.
    /// @param pheromoneIds Array of pheromone IDs to prune.
    function cleanupBatch(bytes32[] calldata pheromoneIds) external;

    // ---------------------------------------------------------------
    // View functions
    // ---------------------------------------------------------------

    /// @notice Compute the current decayed intensity of a pheromone.
    ///         Uses fixed-point integer arithmetic (no floating point).
    ///         Formula: intensity_0 * 2^(-(current_block - deposit_block) / effective_half_life)
    /// @param pheromoneId The pheromone to query.
    /// @return intensity Current intensity in basis points (0 = dead).
    function currentIntensity(bytes32 pheromoneId) external view returns (uint64 intensity);

    /// @notice Compute the SINR of a target pheromone against all
    ///         interferers of the same type at the same location.
    ///         SINR = intensity(target) * SCALE / (sum(interferers) + NOISE_FLOOR)
    /// @param pheromoneId The target pheromone.
    /// @return sinr SINR value scaled by 10000 (basis points).
    function sinr(bytes32 pheromoneId) external view returns (uint64 sinr);

    /// @notice Read pheromones near a query location.
    /// @param queryVector 1,280-byte HDC vector.
    /// @param pType Filter by pheromone type.
    /// @param topK Number of results.
    /// @return ids Pheromone IDs.
    /// @return sinrValues SINR values (scaled by 10000).
    function readPheromones(
        bytes calldata queryVector,
        PheromoneType pType,
        uint8 topK
    ) external view returns (bytes32[] memory ids, uint64[] memory sinrValues);
}
```

### ABI

| Function | Inputs | Outputs |
|----------|--------|---------|
| `deposit(bytes,uint8,uint64)` | `(bytes location, uint8 pType, uint64 intensity)` | none |
| `confirm(bytes32)` | `(bytes32 pheromoneId)` | none |
| `cleanup(bytes32)` | `(bytes32 pheromoneId)` | none |
| `cleanupBatch(bytes32[])` | `(bytes32[] pheromoneIds)` | none |
| `currentIntensity(bytes32)` | `(bytes32 pheromoneId)` | `(uint64 intensity)` |
| `sinr(bytes32)` | `(bytes32 pheromoneId)` | `(uint64 sinr)` |
| `readPheromones(bytes,uint8,uint8)` | `(bytes queryVector, uint8 pType, uint8 topK)` | `(bytes32[], uint64[])` |

---

## 4. InsightBoard.sol

### Purpose

The InsightBoard manages the full lifecycle of shared knowledge on-chain.
It is a 7-state FSM that tracks insights from submission through confirmation,
decay, and eventual purging. It interacts with the HDC precompile for vector
storage/search and uses integer-only arithmetic for consensus safety.

### Storage Layout Diagram

```
Slot Assignments for Insight struct (per insightId in mapping):
=============================================================

  mapping(bytes32 => Insight) public insights;
  mapping(bytes32 => mapping(address => bool)) public confirmers;

  Each Insight occupies 5 storage slots:

  Slot 0 (InsightAnchor.vectorHash):
  +------------------------------------------------------------------+
  | bytes32 vectorHash                                    [256 bits]  |
  +------------------------------------------------------------------+

  Slot 1 (InsightAnchor.contentHash):
  +------------------------------------------------------------------+
  | bytes32 contentHash                                   [256 bits]  |
  +------------------------------------------------------------------+

  Slot 2 (InsightAnchor packed fields):
  +--------+----------+------+------+-------+-------------------------+
  | state  | tier     | kind | pad  | publishBlock | author           |
  | 1 byte | 1 byte   | 1 b  | 1 b  | 8 bytes     | 20 bytes        |
  +--------+----------+------+------+-------------+-----------------+
  Note: Solidity packs right-to-left in a slot. The exact layout
  depends on struct field ordering. The anchor struct below uses
  the ordering that achieves tight packing.

  Slot 3 (confirmation counters, packed):
  +-------------------+---------------------+------------------------+
  | confirmsSince     | lastConfirmedBlock  | confirmations          |
  | Challenge (8 B)   | (8 bytes)           | (8 bytes)              |
  +-------------------+---------------------+------------------------+
  Total: 24 bytes -> fits in one 32-byte slot.

  Slot 4 (staked amount):
  +------------------------------------------------------------------+
  | uint256 stakedAmount                                  [256 bits]  |
  +------------------------------------------------------------------+

  Total: 5 slots per insight = 5 x 32 = 160 bytes.
  At 22,100 gas per cold new-slot SSTORE, initial write = ~110,500 gas.
  Warm updates to packed slots = ~2,900 gas each.
```

### State Transition Diagram

```
                              confirm()
                         (SUBMITTED, ACTIVE,
                          DECAYING, CHALLENGED)
                                 |
                  +--------------+--------------+
                  |              |              |
                  v              v              v
(new)---> SUBMITTED -----> ACTIVE <----- DECAYING -----> ARCHIVED -----> PURGED --->(removed)
              |         ^    |  ^            ^               |
              |         |    |  |            |               |
              |    1st  |    |  | re-confirm |          renew()
              |   conf  |    |  | (resets    |          (ARCHIVED
              |         |    |  |  age)      |           -> ACTIVE)
              |         |    |  |            |
              |         |    v  |            |
              |         | CHALLENGED --------+-----> ARCHIVED
              |         |    ^  |                   (no confs for
              |         |    |  |                    2x half-life)
              |         |    |  +-----> ACTIVE
              |         |    |         (5+ new confs
              |         |    |          since challenge)
              |         |    |
              |         | challenge()
              |         | (ACTIVE only)
              |         |
              +---------+--- computeState() decay thresholds:
                              age > 1x HL -> DECAYING
                              age > 5x HL -> ARCHIVED
                              age > 10x HL -> PURGED

  VERIFIED (enum value 1) exists as a forward-compatible placeholder.
  Currently collapsed: SUBMITTED -> ACTIVE on first confirm().

  Terminal: PURGED -> (removed) via purge() transaction.
```

### Complete Solidity Code

```solidity
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.20;

import {IInsightBoard} from "./IInsightBoard.sol";
import {HdcLib} from "./HdcPrecompile.sol";

/// @title InsightBoard
/// @notice 7-state FSM for on-chain knowledge lifecycle.
/// @dev All arithmetic is integer. No floating point anywhere.
///      Uses the HDC precompile at 0x09 for vector storage and search.
contract InsightBoard is IInsightBoard {
    // ---------------------------------------------------------------
    // Constants
    // ---------------------------------------------------------------

    /// @dev Hamming distance below which two vectors are near-duplicates.
    ///      512 / 10,240 = 5% bit difference = similarity > 0.95.
    uint16 public constant DUPLICATE_THRESHOLD = 512;

    /// @dev Minimum stake required per insight (in native token).
    uint256 public constant MIN_STAKE = 0.01 ether;

    /// @dev Number of new confirmations needed to resolve a CHALLENGED state.
    uint64 public constant CHALLENGE_RESOLUTION_CONFS = 5;

    /// @dev Hamming distance threshold for anti-knowledge resonance.
    ///      1,024 / 10,240 = 10% bit difference = similarity > 0.90.
    uint16 public constant RESONANCE_THRESHOLD = 1024;

    // ---------------------------------------------------------------
    // Structs
    // ---------------------------------------------------------------

    /// @dev On-chain anchor for an insight. Packed into 3 storage slots.
    ///      Field ordering is chosen for tight Solidity packing.
    struct InsightAnchor {
        bytes32 vectorHash;      // slot 0: keccak256 of the full 1,280-byte vector
        bytes32 contentHash;     // slot 1: keccak256 of the content payload
        address author;          // slot 2: 20 bytes ─┐
        uint64  publishBlock;    //          8 bytes  ─┤ packed into slot 2
        uint8   kind;            //          1 byte   ─┤ (31 bytes total)
        uint8   tier;            //          1 byte   ─┤
        uint8   state;           //          1 byte   ─┘
    }

    struct Insight {
        InsightAnchor anchor;           // 3 storage slots
        uint64 confirmations;           // ─┐
        uint64 lastConfirmedBlock;      // ─┤ slot 3 (24 bytes packed)
        uint64 confirmsSinceChallenge;  // ─┘
        uint256 stakedAmount;           // slot 4
    }

    // ---------------------------------------------------------------
    // Storage
    // ---------------------------------------------------------------

    /// @notice All insights by their unique ID.
    mapping(bytes32 => Insight) public insights;

    /// @notice Tracks which addresses have confirmed each insight.
    ///         Prevents double-confirmation.
    mapping(bytes32 => mapping(address => bool)) public confirmers;

    // ---------------------------------------------------------------
    // Write Functions
    // ---------------------------------------------------------------

    /// @inheritdoc IInsightBoard
    function submit(
        Kind kind,
        bytes calldata vector,
        bytes calldata content
    ) external payable returns (bytes32 insightId) {
        // --- Input validation ---
        require(vector.length == 1280, "InsightBoard: invalid vector size");
        require(msg.value >= MIN_STAKE, "InsightBoard: insufficient stake");

        bytes32 vectorHash = keccak256(vector);

        // --- Duplicate check via HDC precompile ---
        (bytes32[] memory similar, uint16[] memory distances) =
            HdcLib.searchSimilar(vector, 5);
        for (uint256 i = 0; i < similar.length; i++) {
            require(
                distances[i] > DUPLICATE_THRESHOLD,
                "InsightBoard: too similar to existing insight"
            );
        }

        // --- Compute unique ID ---
        insightId = keccak256(
            abi.encodePacked(vectorHash, msg.sender, block.number)
        );

        // BUG GUARD: Ensure this ID is not already taken.
        // Extremely unlikely with keccak256, but defensive.
        require(
            insights[insightId].anchor.publishBlock == 0,
            "InsightBoard: ID collision"
        );

        // --- Write state (CEI: state changes BEFORE external calls) ---
        insights[insightId] = Insight({
            anchor: InsightAnchor({
                vectorHash: vectorHash,
                contentHash: keccak256(content),
                author: msg.sender,
                publishBlock: uint64(block.number),
                kind: uint8(kind),
                tier: uint8(Tier.TRANSIENT),
                state: uint8(State.SUBMITTED)
            }),
            confirmations: 0,
            lastConfirmedBlock: uint64(block.number),
            confirmsSinceChallenge: 0,
            stakedAmount: msg.value
        });

        // --- Store vector in HDC precompile index ---
        HdcLib.storeVector(insightId, vector);

        // --- Emit event AFTER state changes (CEI pattern) ---
        emit InsightPublished(
            insightId,
            vectorHash,
            msg.sender,
            vector,
            content,
            uint8(kind),
            uint8(Tier.TRANSIENT)
        );

        emit InsightStateChanged(
            insightId,
            type(uint8).max, // no previous state (new entry)
            uint8(State.SUBMITTED)
        );
    }

    /// @inheritdoc IInsightBoard
    /// @dev CRITICAL: Must accept 4 states (SUBMITTED, ACTIVE, DECAYING, CHALLENGED).
    ///      DO NOT use `require(state == State.ACTIVE)` alone.
    ///      Uses lastConfirmedBlock for age computation, NOT publishBlock.
    function confirm(bytes32 insightId) external {
        // --- Prevent double-confirmation ---
        require(
            !confirmers[insightId][msg.sender],
            "InsightBoard: already confirmed"
        );

        Insight storage insight = insights[insightId];

        // --- Existence check ---
        // A non-existent insight has publishBlock == 0 and state == 0 (SUBMITTED).
        // Without this guard, confirming a non-existent ID writes phantom state.
        require(
            insight.anchor.publishBlock != 0,
            "InsightBoard: insight does not exist"
        );

        // --- State check: accept 4 confirmable states ---
        uint8 currentState = insight.anchor.state;
        require(
            currentState == uint8(State.SUBMITTED) ||
            currentState == uint8(State.ACTIVE) ||
            currentState == uint8(State.DECAYING) ||
            currentState == uint8(State.CHALLENGED),
            "InsightBoard: not confirmable"
        );

        // --- Update confirmation data ---
        confirmers[insightId][msg.sender] = true;
        insight.confirmations++;
        insight.lastConfirmedBlock = uint64(block.number);

        // --- State transitions on confirmation ---
        uint8 oldState = currentState;
        if (
            currentState == uint8(State.SUBMITTED) ||
            currentState == uint8(State.DECAYING)
        ) {
            // SUBMITTED -> ACTIVE (first confirmation)
            // DECAYING -> ACTIVE (re-confirmation recovery)
            insight.anchor.state = uint8(State.ACTIVE);
            insight.confirmsSinceChallenge = 0;
        } else if (currentState == uint8(State.CHALLENGED)) {
            // Track confirmations received while CHALLENGED
            insight.confirmsSinceChallenge++;
            if (insight.confirmsSinceChallenge >= CHALLENGE_RESOLUTION_CONFS) {
                // 5+ new confirmations resolve the challenge
                insight.anchor.state = uint8(State.ACTIVE);
                insight.confirmsSinceChallenge = 0;
            }
        }
        // State.ACTIVE: no state change, just refresh lastConfirmedBlock.

        // --- Tier promotion based on total confirmations ---
        _promoteTier(insight);

        // --- Emit events AFTER all state changes (CEI) ---
        emit InsightConfirmed(insightId, msg.sender, insight.confirmations);

        if (insight.anchor.state != oldState) {
            emit InsightStateChanged(insightId, oldState, insight.anchor.state);
        }
    }

    /// @inheritdoc IInsightBoard
    /// @dev Only ACTIVE insights can be challenged.
    ///      The anti-knowledge insight must exist, be of kind ANTI_KNOWLEDGE,
    ///      and have HDC resonance (Hamming distance < RESONANCE_THRESHOLD)
    ///      with the target insight's vector.
    function challenge(
        bytes32 targetInsightId,
        bytes32 antiKnowledgeId
    ) external {
        Insight storage target = insights[targetInsightId];
        Insight storage anti = insights[antiKnowledgeId];

        // --- Existence checks ---
        require(
            target.anchor.publishBlock != 0,
            "InsightBoard: target does not exist"
        );
        require(
            anti.anchor.publishBlock != 0,
            "InsightBoard: anti-knowledge does not exist"
        );

        // --- Target must be ACTIVE ---
        require(
            target.anchor.state == uint8(State.ACTIVE),
            "InsightBoard: target not ACTIVE"
        );

        // --- Anti-knowledge must be of kind ANTI_KNOWLEDGE ---
        require(
            anti.anchor.kind == uint8(Kind.ANTI_KNOWLEDGE),
            "InsightBoard: challenger is not ANTI_KNOWLEDGE"
        );

        // --- HDC resonance check via precompile ---
        // The anti-knowledge vector, when unbound from ANTI_SUBSPACE, must
        // be similar to the target vector. We check the stored vectorHashes
        // and rely on the precompile's hamming function for the distance.
        // NOTE: The full vector-level resonance check requires off-chain
        // computation. On-chain, we verify that both insights exist and
        // the anti-knowledge has the correct kind. A more complete on-chain
        // check would require storing the vectors in contract storage
        // (expensive) or a precompile call with both IDs.

        // --- State transition: ACTIVE -> CHALLENGED ---
        uint8 oldState = target.anchor.state;
        target.anchor.state = uint8(State.CHALLENGED);
        target.confirmsSinceChallenge = 0;

        // --- Emit events AFTER state changes (CEI) ---
        emit InsightChallenged(targetInsightId, antiKnowledgeId, msg.sender);
        emit InsightStateChanged(targetInsightId, oldState, uint8(State.CHALLENGED));
    }

    /// @inheritdoc IInsightBoard
    function renew(bytes32 insightId) external payable {
        Insight storage insight = insights[insightId];

        require(
            insight.anchor.publishBlock != 0,
            "InsightBoard: insight does not exist"
        );
        require(
            insight.anchor.state == uint8(State.ARCHIVED),
            "InsightBoard: only ARCHIVED insights can be renewed"
        );
        require(msg.value >= MIN_STAKE, "InsightBoard: insufficient stake");

        uint8 oldState = insight.anchor.state;

        insight.anchor.publishBlock = uint64(block.number);
        insight.lastConfirmedBlock = uint64(block.number);
        insight.anchor.state = uint8(State.ACTIVE);
        insight.stakedAmount += msg.value;

        emit InsightRenewed(insightId, msg.sender);
        emit InsightStateChanged(insightId, oldState, uint8(State.ACTIVE));
    }

    /// @inheritdoc IInsightBoard
    /// @dev CEI pattern: read state -> delete storage -> external call.
    ///      The purge() function sends 10% of stake to the original author.
    ///      The delete triggers SSTORE refund for the caller.
    function purge(bytes32 insightId) external {
        Insight storage insight = insights[insightId];

        require(
            insight.anchor.publishBlock != 0,
            "InsightBoard: insight does not exist"
        );
        require(
            computeState(insightId) == State.PURGED,
            "InsightBoard: not yet purgeable"
        );

        // --- Cache values before deletion ---
        address author = insight.anchor.author;
        uint256 legacy = insight.stakedAmount / 10; // 10% to original author

        // --- Remove from precompile index ---
        HdcLib.deleteVector(insightId);

        // --- Clear storage (triggers SSTORE refund) ---
        delete insights[insightId];

        // --- Emit BEFORE external ETH transfer (event is pure log, safe) ---
        emit InsightPurged(insightId, msg.sender);

        // --- Transfer legacy to original author ---
        if (legacy > 0) {
            (bool ok, ) = author.call{value: legacy}("");
            require(ok, "InsightBoard: legacy transfer failed");
        }
    }

    // ---------------------------------------------------------------
    // View Functions
    // ---------------------------------------------------------------

    /// @inheritdoc IInsightBoard
    /// @dev CRITICAL: Uses age relative to lastConfirmedBlock (NOT publishBlock).
    ///      This ensures re-confirmation resets the decay clock.
    function computeState(bytes32 insightId) public view returns (State) {
        Insight storage insight = insights[insightId];

        // Non-existent insights return PURGED (safe default for purge guard)
        if (insight.anchor.publishBlock == 0) {
            return State.PURGED;
        }

        // --- Age computation from lastConfirmedBlock ---
        uint64 age = uint64(block.number) - insight.lastConfirmedBlock;
        uint64 tierMult = _tierMultiplier(Tier(insight.anchor.tier));
        uint64 kindHL = _kindHalfLife(Kind(insight.anchor.kind));
        uint64 effectiveHL = kindHL * tierMult;

        // --- Decay thresholds take precedence over stored state ---
        if (age > effectiveHL * 10) {
            return State.PURGED;
        }

        // CHALLENGED entries use 2x half-life for ARCHIVED threshold.
        // Silence on a challenged entry = tacit agreement with challenge.
        if (
            insight.anchor.state == uint8(State.CHALLENGED) &&
            age > effectiveHL * 2
        ) {
            return State.ARCHIVED;
        }

        if (age > effectiveHL * 5) {
            return State.ARCHIVED;
        }

        if (age > effectiveHL) {
            return State.DECAYING;
        }

        // --- Within active lifetime: preserve stored state ---
        return State(insight.anchor.state);
    }

    /// @inheritdoc IInsightBoard
    function searchSimilar(
        bytes calldata queryVector,
        uint8 topK
    ) external view returns (bytes32[] memory, uint16[] memory) {
        return HdcLib.searchSimilar(queryVector, topK);
    }

    // ---------------------------------------------------------------
    // Internal Helpers
    // ---------------------------------------------------------------

    /// @dev Kind-specific base half-lives in blocks (~0.4s/block).
    ///      Conversion: hours * 3600 / 0.4 = hours * 9000 blocks/hour.
    function _kindHalfLife(Kind kind) internal pure returns (uint64) {
        if (kind == Kind.INSIGHT)        return   648_000; // 72h
        if (kind == Kind.HEURISTIC)      return 1_512_000; // 168h
        if (kind == Kind.ANTI_KNOWLEDGE) return 3_024_000; // 336h
        if (kind == Kind.WARNING)        return   432_000; // 48h
        if (kind == Kind.CAUSAL_LINK)    return 2_160_000; // 240h
        if (kind == Kind.STRATEGY)       return 1_080_000; // 120h
        revert("InsightBoard: unknown kind");
    }

    /// @dev Tier multipliers for effective half-life.
    function _tierMultiplier(Tier tier) internal pure returns (uint64) {
        if (tier == Tier.TRANSIENT)    return 1;
        if (tier == Tier.WORKING)     return 3;
        if (tier == Tier.CONSOLIDATED) return 7;
        if (tier == Tier.PERSISTENT)  return 10;
        revert("InsightBoard: unknown tier");
    }

    /// @dev Promote tier based on total confirmation count.
    ///      Promotions are monotonic (never demote).
    function _promoteTier(Insight storage insight) internal {
        uint64 confs = insight.confirmations;
        if (confs >= 25 && insight.anchor.tier < uint8(Tier.PERSISTENT)) {
            insight.anchor.tier = uint8(Tier.PERSISTENT);
        } else if (confs >= 10 && insight.anchor.tier < uint8(Tier.CONSOLIDATED)) {
            insight.anchor.tier = uint8(Tier.CONSOLIDATED);
        } else if (confs >= 3 && insight.anchor.tier < uint8(Tier.WORKING)) {
            insight.anchor.tier = uint8(Tier.WORKING);
        }
    }
}
```

### Gas Analysis

| Function | Estimated Gas | Breakdown |
|----------|---------------|-----------|
| `submit()` | ~145,000 | Calldata (~15,600) + precompile duplicate check (~50,000) + 5 SSTORE slots (~110,500 cold) + event LOG3 (~13,350) |
| `confirm()` | ~28,000-35,000 | Cold SLOAD (~2,100) + SSTORE confirmer mapping (~22,100 new) + SSTORE counter update (~2,900 warm) + tier promotion (~2,900 warm, if triggered) + event (~375+) |
| `challenge()` | ~15,000-20,000 | 2x cold SLOAD (~4,200) + SSTORE state change (~2,900 warm) + SSTORE confirmsSinceChallenge (~2,900 warm) + events |
| `renew()` | ~12,000-15,000 | Cold SLOAD (~2,100) + 3x SSTORE warm (~8,700) + event |
| `purge()` | ~20,000-30,000 | Cold SLOAD (~2,100) + precompile deleteVector (~5,000) + storage delete (refund ~4,800/slot x 5 = ~24,000 refund) + ETH transfer (~2,300) + event. Net cost offset by refund. |
| `computeState()` | ~5,000-8,000 | Cold SLOAD (~2,100) + integer arithmetic (~200) |
| `searchSimilar()` | ~50,000 | Delegated entirely to precompile (view, free for caller via eth_call) |

### Security Checklist

1. **Reentrancy:** The only external ETH transfer is in `purge()`, which
   follows CEI -- state is deleted before the `call{value}`. The `delete`
   zeroes storage, so a reentrant call to `purge()` fails the existence check.

2. **Integer overflow:** Solidity 0.8+ has built-in overflow checks on all
   arithmetic. The `effectiveHL * 10` multiplication in `computeState()` could
   theoretically overflow uint64 for very large half-lives, but the maximum
   value is `3,024,000 * 10 * 10 = 302,400,000` which is well within uint64
   range (max ~1.8 x 10^19).

3. **Access control:**
   - `submit()` -- open to all (permissionless, guarded by stake).
   - `confirm()` -- open to all, guarded by double-confirmation check.
   - `challenge()` -- open to all, guarded by state/kind checks.
   - `renew()` -- open to all, guarded by state check + stake.
   - `purge()` -- open to all, guarded by computeState() == PURGED.

4. **Front-running:**
   - `submit()` could be front-run to steal an insight ID. Mitigation: the
     insightId includes `msg.sender`, so a front-runner gets a different ID.
   - `confirm()` is not front-runnable in a meaningful way (order of
     confirmation does not matter).
   - No commit-reveal needed for the current design.

5. **Proxy implications:** If deployed behind a proxy, `msg.sender` in
   `submit()` will be the proxy caller, not the end user. If delegatecall
   is used, `msg.sender` is preserved. Document the proxy pattern chosen.

---

## 5. PheromoneRegistry.sol

### Purpose

Stigmergic coordination through digital pheromones. Agents deposit pheromones
at locations in HDC vector space. Pheromones decay exponentially using
fixed-point integer arithmetic. The SINR model prevents signal flooding.
The alpha paradox reduces half-life on confirmation.

### Storage Layout Diagram

```
Slot Assignments for PheromoneDeposit struct:
=============================================

  mapping(bytes32 => PheromoneDeposit) public pheromones;
  mapping(bytes32 => bytes32[]) internal _locationPheromones;
  mapping(bytes32 => mapping(address => bool)) internal _confirmers;

  Each PheromoneDeposit occupies 3 storage slots:

  Slot 0 (locationHash):
  +------------------------------------------------------------------+
  | bytes32 locationHash                                  [256 bits]  |
  +------------------------------------------------------------------+

  Slot 1 (packed fields):
  +----------+-----------+-------------+----------+-------------------+
  | pType    | confirms  | depositBlock| intensity| depositor         |
  | 1 byte   | 2 bytes   | 8 bytes     | 8 bytes  | 20 bytes          |
  +----------+-----------+-------------+----------+-------------------+
  Note: Solidity packs right-to-left. Ordering below achieves
  tight packing: depositor(20) + intensity(8) + depositBlock(8) +
  confirmationCount(2) + pType(1) = 39 bytes. This exceeds 32 bytes,
  so Solidity will use 2 slots for non-bytes32 fields.

  Adjusted layout (2 data slots):

  Slot 1 (depositor + intensity + depositBlock):
  +-------------------+------------------+---------------------------+
  | depositBlock (8B) | intensity (8B)   | depositor (20B)           |
  +-------------------+------------------+---------------------------+

  Slot 2 (confirmation count + pheromone type):
  +---------------------------+-------+------+-----------------------+
  | padding                   | pType | confirmationCount           |
  | (28 bytes)                | (1B)  | (2 bytes)                   |
  +---------------------------+-------+-----------------------------+

  Total: 3 slots per pheromone deposit.
```

### Complete Solidity Code

```solidity
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.20;

import {IPheromoneRegistry} from "./IPheromoneRegistry.sol";
import {HdcLib} from "./HdcPrecompile.sol";

/// @title PheromoneRegistry
/// @notice Stigmergic coordination via digital pheromones with exponential
///         decay (fixed-point integer math) and SINR interference.
/// @dev All on-chain arithmetic is integer. No floating point.
///      Decay is computed at READ TIME -- no storage updates needed for decay.
contract PheromoneRegistry is IPheromoneRegistry {
    // ---------------------------------------------------------------
    // Constants
    // ---------------------------------------------------------------

    /// @dev Minimum stake per pheromone deposit.
    uint256 public constant MIN_PHEROMONE_STAKE = 0.001 ether;

    /// @dev Minimum initial intensity.
    uint64 public constant MIN_INTENSITY = 100;

    /// @dev Maximum initial intensity.
    uint64 public constant MAX_INTENSITY = 10_000;

    /// @dev Death threshold: intensity below this is considered dead.
    ///      In the same units as initial intensity. A dead pheromone
    ///      contributes SINR < 0.1 (below actionable threshold of 0.5).
    uint64 public constant DEATH_THRESHOLD = 1;

    /// @dev SINR noise floor. Prevents division by zero and models
    ///      background uncertainty.
    uint64 public constant NOISE_FLOOR = 10;

    /// @dev Scaling factor for fixed-point SINR output (basis points).
    uint64 public constant SINR_SCALE = 10_000;

    /// @dev Hamming distance threshold for location proximity.
    ///      Two pheromones "overlap" when their location vectors have
    ///      distance below this value. 1,024 / 10,240 = similarity > 0.9.
    uint16 public constant PROXIMITY_THRESHOLD = 1024;

    /// @dev Fixed-point scaling for decay computation (2^16 = 65536).
    ///      Used as the denominator in the fractional part of the
    ///      exponential decay approximation.
    uint64 internal constant FP_SCALE = 65536;

    // ---------------------------------------------------------------
    // Structs
    // ---------------------------------------------------------------

    struct PheromoneDeposit {
        bytes32 locationHash;       // keccak256 of the location vector
        address depositor;          // ─┐
        uint64  intensity;          // ─┤ packed
        uint64  depositBlock;       // ─┘
        uint16  confirmationCount;  // ─┐
        uint8   pType;              // ─┘ packed
    }

    // ---------------------------------------------------------------
    // Storage
    // ---------------------------------------------------------------

    /// @notice All pheromone deposits by unique ID.
    mapping(bytes32 => PheromoneDeposit) public pheromones;

    /// @notice Maps locationHash -> array of pheromoneIds at that location.
    ///         Used for SINR computation (finding interferers).
    mapping(bytes32 => bytes32[]) internal _locationPheromones;

    /// @notice Tracks which addresses have confirmed each pheromone.
    mapping(bytes32 => mapping(address => bool)) internal _confirmers;

    // ---------------------------------------------------------------
    // Half-life lookup
    // ---------------------------------------------------------------

    /// @dev Base half-lives in blocks per pheromone type.
    function _baseHalfLife(PheromoneType pType) internal pure returns (uint64) {
        if (pType == PheromoneType.THREAT)      return 100;
        if (pType == PheromoneType.OPPORTUNITY)  return 250;
        if (pType == PheromoneType.WISDOM)       return 1000;
        revert("PheromoneRegistry: unknown type");
    }

    // ---------------------------------------------------------------
    // Write Functions
    // ---------------------------------------------------------------

    /// @inheritdoc IPheromoneRegistry
    function deposit(
        bytes calldata location,
        PheromoneType pType,
        uint64 intensity
    ) external payable {
        require(location.length == 1280, "PheromoneRegistry: invalid vector size");
        require(
            intensity >= MIN_INTENSITY && intensity <= MAX_INTENSITY,
            "PheromoneRegistry: intensity out of range"
        );
        require(
            msg.value >= MIN_PHEROMONE_STAKE,
            "PheromoneRegistry: insufficient stake"
        );

        bytes32 locationHash = keccak256(location);
        bytes32 pheromoneId = keccak256(
            abi.encodePacked(locationHash, msg.sender, block.number, uint8(pType))
        );

        // Existence guard
        require(
            pheromones[pheromoneId].depositBlock == 0,
            "PheromoneRegistry: ID collision"
        );

        // --- Write state ---
        pheromones[pheromoneId] = PheromoneDeposit({
            locationHash: locationHash,
            depositor: msg.sender,
            intensity: intensity,
            depositBlock: uint64(block.number),
            confirmationCount: 0,
            pType: uint8(pType)
        });

        _locationPheromones[locationHash].push(pheromoneId);

        // --- Emit event AFTER state changes (CEI) ---
        emit PheromoneDeposited(
            pheromoneId,
            locationHash,
            msg.sender,
            uint8(pType),
            intensity,
            uint64(block.number)
        );
    }

    /// @inheritdoc IPheromoneRegistry
    /// @dev Alpha paradox: confirmation REDUCES half-life.
    ///      new_half_life = base_half_life / (1 + confirmation_count)
    ///      This is a harmonic reduction. Each confirmation roughly halves
    ///      the remaining persistence.
    function confirm(bytes32 pheromoneId) external {
        PheromoneDeposit storage p = pheromones[pheromoneId];
        require(p.depositBlock != 0, "PheromoneRegistry: does not exist");
        require(
            !_confirmers[pheromoneId][msg.sender],
            "PheromoneRegistry: already confirmed"
        );

        // --- Check that pheromone is still alive ---
        require(
            _computeIntensity(p) >= DEATH_THRESHOLD,
            "PheromoneRegistry: pheromone is dead"
        );

        _confirmers[pheromoneId][msg.sender] = true;
        p.confirmationCount++;

        // The reduced half-life takes effect at the CURRENT block.
        // Reset depositBlock so decay restarts from now with shorter half-life.
        // Preserve current intensity as the new baseline.
        uint64 currentInt = _computeIntensity(p);
        p.intensity = currentInt;
        p.depositBlock = uint64(block.number);

        uint64 newHL = _effectiveHalfLife(p);

        emit PheromoneConfirmed(
            pheromoneId,
            msg.sender,
            p.confirmationCount,
            newHL
        );
    }

    /// @inheritdoc IPheromoneRegistry
    function cleanup(bytes32 pheromoneId) external {
        PheromoneDeposit storage p = pheromones[pheromoneId];
        require(p.depositBlock != 0, "PheromoneRegistry: does not exist");
        require(
            _computeIntensity(p) < DEATH_THRESHOLD,
            "PheromoneRegistry: not dead yet"
        );

        bytes32 locHash = p.locationHash;

        // --- Remove from location index ---
        _removeFromLocationIndex(locHash, pheromoneId);

        // --- Clear storage (triggers SSTORE refund) ---
        delete pheromones[pheromoneId];

        emit PheromonePruned(pheromoneId, msg.sender);
    }

    /// @inheritdoc IPheromoneRegistry
    function cleanupBatch(bytes32[] calldata pheromoneIds) external {
        for (uint256 i = 0; i < pheromoneIds.length; i++) {
            bytes32 pid = pheromoneIds[i];
            PheromoneDeposit storage p = pheromones[pid];
            if (p.depositBlock == 0) continue; // skip non-existent
            if (_computeIntensity(p) >= DEATH_THRESHOLD) continue; // skip alive

            bytes32 locHash = p.locationHash;
            _removeFromLocationIndex(locHash, pid);
            delete pheromones[pid];
            emit PheromonePruned(pid, msg.sender);
        }
    }

    // ---------------------------------------------------------------
    // View Functions
    // ---------------------------------------------------------------

    /// @inheritdoc IPheromoneRegistry
    function currentIntensity(bytes32 pheromoneId) external view returns (uint64) {
        PheromoneDeposit storage p = pheromones[pheromoneId];
        if (p.depositBlock == 0) return 0;
        return _computeIntensity(p);
    }

    /// @inheritdoc IPheromoneRegistry
    /// @dev SINR = intensity(target) * SINR_SCALE / (sum(interferers) + NOISE_FLOOR)
    ///      All arithmetic is integer. Result is in basis points.
    function sinr(bytes32 pheromoneId) external view returns (uint64) {
        PheromoneDeposit storage target = pheromones[pheromoneId];
        if (target.depositBlock == 0) return 0;

        uint64 targetIntensity = _computeIntensity(target);
        if (targetIntensity < DEATH_THRESHOLD) return 0;

        // Sum intensities of all interferers at the same location
        // with the same pheromone type.
        bytes32 locHash = target.locationHash;
        bytes32[] storage atLocation = _locationPheromones[locHash];
        uint64 sumInterferers = 0;

        for (uint256 i = 0; i < atLocation.length; i++) {
            bytes32 otherId = atLocation[i];
            if (otherId == pheromoneId) continue; // skip self

            PheromoneDeposit storage other = pheromones[otherId];
            if (other.depositBlock == 0) continue; // pruned
            if (other.pType != target.pType) continue; // different type

            uint64 otherInt = _computeIntensity(other);
            if (otherInt >= DEATH_THRESHOLD) {
                sumInterferers += otherInt;
            }
        }

        // SINR = target * SCALE / (interference + noise)
        return uint64(
            (uint256(targetIntensity) * uint256(SINR_SCALE)) /
            (uint256(sumInterferers) + uint256(NOISE_FLOOR))
        );
    }

    /// @inheritdoc IPheromoneRegistry
    /// @dev Delegates to HDC precompile for location search, then
    ///      computes SINR for each result.
    function readPheromones(
        bytes calldata queryVector,
        PheromoneType pType,
        uint8 topK
    ) external view returns (bytes32[] memory ids, uint64[] memory sinrValues) {
        // For a full implementation, this would search the precompile index
        // for pheromone location vectors near the query, then compute SINR
        // for each. Simplified version: iterate known locations.
        // The full implementation requires the precompile to maintain a
        // separate pheromone location index or reuse the insight index
        // with a type filter.
        //
        // Placeholder: return empty arrays. The implementing agent should
        // wire this to the precompile's searchSimilar with a pheromone-
        // specific index namespace.
        ids = new bytes32[](0);
        sinrValues = new uint64[](0);
    }

    // ---------------------------------------------------------------
    // Internal: Fixed-Point Decay
    // ---------------------------------------------------------------

    /// @dev Compute the effective half-life after alpha paradox reduction.
    ///      new_half_life = base_half_life / (1 + confirmation_count)
    function _effectiveHalfLife(
        PheromoneDeposit storage p
    ) internal view returns (uint64) {
        uint64 base = _baseHalfLife(PheromoneType(p.pType));
        return base / (1 + uint64(p.confirmationCount));
    }

    /// @dev Compute decayed intensity using fixed-point integer arithmetic.
    ///      Formula: intensity_0 * 2^(-(age) / half_life)
    ///
    ///      Decomposition:
    ///        Let q = age / half_life  (integer division = number of full halvings)
    ///        Let r = age % half_life  (remainder)
    ///        Then 2^(-age/hl) = 2^(-q) * 2^(-r/hl)
    ///
    ///      2^(-q) is implemented as right-shift by q.
    ///      2^(-r/hl) is in [0.5, 1.0) and approximated via linear interpolation:
    ///        2^(-r/hl) ~ (FP_SCALE - (FP_SCALE/2 * r / hl)) / FP_SCALE
    ///      This is a first-order Taylor approximation that is accurate to within
    ///      ~2% for r/hl in [0, 1). For pheromone use cases (fast decay, low
    ///      precision requirements), this is sufficient.
    ///
    ///      CONSENSUS SAFETY: All operations are integer division and
    ///      multiplication. No floating point. All validators produce
    ///      identical results.
    function _computeIntensity(
        PheromoneDeposit storage p
    ) internal view returns (uint64) {
        uint64 age = uint64(block.number) - p.depositBlock;
        uint64 hl = _effectiveHalfLife(p);

        if (hl == 0) return 0; // degenerate case: infinite confirmations

        uint64 q = age / hl;  // number of full halvings
        uint64 r = age % hl;  // remainder

        // After 20+ halvings, intensity is < 1/1,000,000 of original.
        // Return 0 to avoid unnecessary computation.
        if (q >= 20) return 0;

        // Integer part: right-shift by q halvings
        uint64 integerDecay = p.intensity >> q;
        if (integerDecay == 0) return 0;

        // Fractional part: linear interpolation of 2^(-r/hl)
        // 2^(-x) ~ 1 - x * ln(2) for small x, but we use a better
        // piecewise approximation:
        //   2^(-r/hl) ~ (2*hl - r) / (2*hl)
        // This maps r=0 -> 1.0, r=hl -> 0.5, which are the exact values.
        // The linear interpolation between these two exact endpoints has
        // maximum error of ~5.7% at r=hl/2 (true value 0.707, approx 0.75).
        uint64 numerator = 2 * hl - r;
        uint64 denominator = 2 * hl;

        // Guard against overflow: use uint128 for intermediate
        uint128 result = (uint128(integerDecay) * uint128(numerator)) / uint128(denominator);

        // Clamp to uint64
        if (result > type(uint64).max) return type(uint64).max;
        return uint64(result);
    }

    // ---------------------------------------------------------------
    // Internal: Location Index Management
    // ---------------------------------------------------------------

    /// @dev Remove a pheromone ID from its location's index array.
    ///      Swap-and-pop for O(1) removal.
    function _removeFromLocationIndex(
        bytes32 locationHash,
        bytes32 pheromoneId
    ) internal {
        bytes32[] storage arr = _locationPheromones[locationHash];
        for (uint256 i = 0; i < arr.length; i++) {
            if (arr[i] == pheromoneId) {
                arr[i] = arr[arr.length - 1];
                arr.pop();
                return;
            }
        }
    }
}
```

### Gas Analysis

| Function | Estimated Gas | Breakdown |
|----------|---------------|-----------|
| `deposit()` | ~80,000 | Calldata (~15,600) + 3 SSTORE slots (~66,300) + location array push (~22,100) + event |
| `confirm()` | ~25,000-30,000 | SLOAD (~2,100) + SSTORE confirmer (~22,100) + SSTORE counter + intensity + depositBlock (~5,800 warm) + event |
| `cleanup()` | ~15,000-20,000 | SLOAD (~2,100) + storage delete (refund ~14,400 for 3 slots) + array removal + event. Net very low after refund. |
| `cleanupBatch()` | ~10,000 + 5,000/item | Base tx cost + per-item cleanup. Amortizes 21,000 base gas over many items. |
| `currentIntensity()` | ~3,000-5,000 | SLOAD (~2,100) + integer math (~200) |
| `sinr()` | ~3,000 + 2,500/interferer | SLOAD target + loop over location array with SLOAD per interferer |
| `readPheromones()` | ~50,000+ | Depends on precompile search + SINR per result |

### Decay Formula: Worked Example

Starting parameters:
- Type: THREAT (base half-life = 100 blocks)
- Initial intensity: 1,000
- 2 confirmations -> effective half-life = 100 / (1+2) = 33 blocks

| Block Offset | q (halvings) | r (remainder) | Computed Intensity |
|-------------|-------------|---------------|-------------------|
| 0 | 0 | 0 | 1,000 |
| 17 | 0 | 17 | 1000 * (66-17)/66 = 742 |
| 33 | 1 | 0 | 1000 >> 1 = 500 |
| 50 | 1 | 17 | 500 * (66-17)/66 = 371 |
| 66 | 2 | 0 | 1000 >> 2 = 250 |
| 330 | 10 | 0 | 1000 >> 10 ~ 0 |

### Security Checklist

1. **Reentrancy:** No external calls that transfer ETH in deposit/confirm/cleanup.
   The `msg.value` in `deposit()` is received, not sent. No reentrancy risk.

2. **Integer overflow:** Solidity 0.8+ checks. The `uint128` intermediate in
   `_computeIntensity` prevents overflow during the multiplication step.

3. **Access control:**
   - `deposit()` -- open, guarded by stake.
   - `confirm()` -- open, guarded by double-confirm check and death check.
   - `cleanup()` -- open, guarded by death threshold check.

4. **Front-running:**
   - `confirm()` could be front-run, but the alpha paradox makes this
     self-defeating: confirming accelerates decay, so front-running a
     confirmation harms the front-runner's own pheromone reading.
   - No commit-reveal needed.

5. **Denial of service:** The `_locationPheromones` array grows unboundedly.
   If an attacker deposits thousands of pheromones at one location, the
   `sinr()` function becomes expensive to call. Mitigation: the `cleanup()`
   function removes dead entries, and gas cost for deposit discourages spam.
   A production deployment should add a per-location cap (e.g., max 100
   active pheromones per locationHash).

6. **Alpha paradox exploitation:** An attacker can confirm a legitimate
   pheromone with Sybil accounts to accelerate its decay. Mitigation: add
   a per-pheromone confirmation rate limit (e.g., max 3 confirmations per
   100 blocks) or require confirmers to have minimum reputation.

---

## 6. Deployment Script

### Foundry Script

```solidity
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.20;

import {Script, console} from "forge-std/Script.sol";
import {InsightBoard} from "../src/InsightBoard.sol";
import {PheromoneRegistry} from "../src/PheromoneRegistry.sol";

/// @title DeployHDC
/// @notice Foundry deployment script for InsightBoard and PheromoneRegistry.
/// @dev Run with:
///      forge script script/DeployHDC.s.sol:DeployHDC \
///        --rpc-url $RPC_URL --broadcast --verify
contract DeployHDC is Script {
    function run() external {
        uint256 deployerKey = vm.envUint("DEPLOYER_PRIVATE_KEY");
        vm.startBroadcast(deployerKey);

        // Deploy InsightBoard
        InsightBoard insightBoard = new InsightBoard();
        console.log("InsightBoard deployed at:", address(insightBoard));

        // Deploy PheromoneRegistry
        PheromoneRegistry pheromoneRegistry = new PheromoneRegistry();
        console.log("PheromoneRegistry deployed at:", address(pheromoneRegistry));

        vm.stopBroadcast();

        // Log deployment summary
        console.log("---");
        console.log("DUPLICATE_THRESHOLD:", insightBoard.DUPLICATE_THRESHOLD());
        console.log("MIN_STAKE:", insightBoard.MIN_STAKE());
        console.log("MIN_PHEROMONE_STAKE:", pheromoneRegistry.MIN_PHEROMONE_STAKE());
    }
}
```

### Deployment Prerequisites

1. The HDC precompile must be available at address `0x09` on the target chain.
   Both contracts will revert on any precompile call if the precompile is not
   deployed. Test with a local daeji devnet that includes the precompile.

2. The deployer account needs sufficient native token for deployment gas
   (~3M gas for InsightBoard, ~2M gas for PheromoneRegistry).

3. No constructor arguments. Both contracts are stateless on deployment.

---

## 7. Test Scenarios

### Foundry Test File Structure

```
test/
  InsightBoard.t.sol
  PheromoneRegistry.t.sol
  HdcLib.t.sol
  mocks/
    MockHdcPrecompile.sol
```

### InsightBoard Test Scenarios

```solidity
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.20;

import {Test, console} from "forge-std/Test.sol";
import {InsightBoard} from "../src/InsightBoard.sol";
import {IInsightBoard} from "../src/IInsightBoard.sol";

contract InsightBoardTest is Test {
    InsightBoard board;

    // NOTE: Tests require a mock HDC precompile at 0x09.
    // Use vm.etch() to deploy mock bytecode at the precompile address.

    function setUp() public {
        board = new InsightBoard();
        // Deploy mock precompile at 0x09
        // vm.etch(address(0x09), mockPrecompileBytecode);
    }

    // --- submit() tests ---

    /// @dev Happy path: submit an insight with valid vector and stake.
    function test_submit_happyPath() public { /* ... */ }

    /// @dev Revert: vector is not 1280 bytes.
    function test_submit_invalidVectorSize() public { /* ... */ }

    /// @dev Revert: stake is below MIN_STAKE.
    function test_submit_insufficientStake() public { /* ... */ }

    /// @dev Revert: vector is too similar to an existing insight.
    function test_submit_duplicateRejected() public { /* ... */ }

    /// @dev Verify: InsightPublished event is emitted with correct params.
    function test_submit_emitsEvent() public { /* ... */ }

    /// @dev Verify: initial state is SUBMITTED.
    function test_submit_initialState() public { /* ... */ }

    /// @dev Verify: insightId includes msg.sender (front-run protection).
    function test_submit_idIncludesSender() public { /* ... */ }

    // --- confirm() tests ---

    /// @dev Happy path: first confirmation transitions SUBMITTED -> ACTIVE.
    function test_confirm_submittedToActive() public { /* ... */ }

    /// @dev Confirm an ACTIVE insight (refresh only, no state change).
    function test_confirm_activeRefresh() public { /* ... */ }

    /// @dev Confirm a DECAYING insight -> transitions to ACTIVE.
    function test_confirm_decayingToActive() public { /* ... */ }

    /// @dev Confirm a CHALLENGED insight: 4 confs -> still CHALLENGED.
    function test_confirm_challengedNotResolved() public { /* ... */ }

    /// @dev Confirm a CHALLENGED insight: 5 confs -> ACTIVE.
    function test_confirm_challengedResolved() public { /* ... */ }

    /// @dev Revert: double confirmation by the same address.
    function test_confirm_doubleConfirmReverts() public { /* ... */ }

    /// @dev Revert: confirm a non-existent insightId.
    function test_confirm_nonExistentReverts() public { /* ... */ }

    /// @dev Revert: confirm an ARCHIVED insight.
    function test_confirm_archivedReverts() public { /* ... */ }

    /// @dev Revert: confirm a PURGED insight.
    function test_confirm_purgedReverts() public { /* ... */ }

    /// @dev Verify: lastConfirmedBlock is updated on confirmation.
    function test_confirm_updatesLastConfirmedBlock() public { /* ... */ }

    /// @dev Verify: tier promotion at 3 confirmations -> WORKING.
    function test_confirm_tierPromotionWorking() public { /* ... */ }

    /// @dev Verify: tier promotion at 10 confirmations -> CONSOLIDATED.
    function test_confirm_tierPromotionConsolidated() public { /* ... */ }

    /// @dev Verify: tier promotion at 25 confirmations -> PERSISTENT.
    function test_confirm_tierPromotionPersistent() public { /* ... */ }

    /// @dev Verify: tier never demotes (monotonic promotion).
    function test_confirm_tierNeverDemotes() public { /* ... */ }

    // --- challenge() tests ---

    /// @dev Happy path: challenge an ACTIVE insight.
    function test_challenge_activeToChallenge() public { /* ... */ }

    /// @dev Revert: challenge a non-ACTIVE insight (e.g., SUBMITTED).
    function test_challenge_nonActiveReverts() public { /* ... */ }

    /// @dev Revert: challenger is not ANTI_KNOWLEDGE kind.
    function test_challenge_wrongKindReverts() public { /* ... */ }

    /// @dev Verify: InsightChallenged + InsightStateChanged events.
    function test_challenge_emitsEvents() public { /* ... */ }

    /// @dev Verify: confirmsSinceChallenge resets to 0 on challenge.
    function test_challenge_resetsConfirmCounter() public { /* ... */ }

    // --- computeState() tests ---

    /// @dev Within 1x half-life: returns stored state.
    function test_computeState_withinHalfLife() public { /* ... */ }

    /// @dev Age > 1x half-life: returns DECAYING.
    function test_computeState_decaying() public { /* ... */ }

    /// @dev Age > 5x half-life: returns ARCHIVED.
    function test_computeState_archived() public { /* ... */ }

    /// @dev Age > 10x half-life: returns PURGED.
    function test_computeState_purged() public { /* ... */ }

    /// @dev CHALLENGED + age > 2x half-life: returns ARCHIVED.
    function test_computeState_challengedArchived() public { /* ... */ }

    /// @dev Uses lastConfirmedBlock, not publishBlock, for age.
    ///      CRITICAL: This is the key invariant. A re-confirmation
    ///      must reset the decay clock.
    function test_computeState_usesLastConfirmedBlock() public {
        // 1. Submit insight at block N.
        // 2. Advance to block N + 1.5 * halfLife (would be DECAYING).
        // 3. Confirm at block N + 1.5 * halfLife.
        // 4. computeState() should return ACTIVE (not DECAYING),
        //    because lastConfirmedBlock was just updated.
    }

    /// @dev Non-existent insight returns PURGED.
    function test_computeState_nonExistent() public { /* ... */ }

    /// @dev Verify tier multiplier affects half-life.
    function test_computeState_tierMultiplier() public { /* ... */ }

    /// @dev Verify kind-specific half-lives.
    function test_computeState_kindHalfLives() public { /* ... */ }

    // --- renew() tests ---

    /// @dev Happy path: renew an ARCHIVED insight -> ACTIVE.
    function test_renew_archivedToActive() public { /* ... */ }

    /// @dev Revert: renew a non-ARCHIVED insight.
    function test_renew_nonArchivedReverts() public { /* ... */ }

    /// @dev Revert: insufficient stake.
    function test_renew_insufficientStakeReverts() public { /* ... */ }

    /// @dev Verify: publishBlock and lastConfirmedBlock both reset.
    function test_renew_resetsBlocks() public { /* ... */ }

    // --- purge() tests ---

    /// @dev Happy path: purge a PURGED insight, 10% to author.
    function test_purge_happyPath() public { /* ... */ }

    /// @dev Revert: purge a non-PURGED insight.
    function test_purge_notPurgeableReverts() public { /* ... */ }

    /// @dev Verify: storage is deleted after purge.
    function test_purge_clearsStorage() public { /* ... */ }

    /// @dev Verify: precompile deleteVector is called.
    function test_purge_deletesFromPrecompile() public { /* ... */ }

    /// @dev Reentrancy: author receives ETH but cannot re-enter purge.
    function test_purge_reentrancySafe() public { /* ... */ }
}
```

### PheromoneRegistry Test Scenarios

```solidity
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.20;

import {Test} from "forge-std/Test.sol";
import {PheromoneRegistry} from "../src/PheromoneRegistry.sol";
import {IPheromoneRegistry} from "../src/IPheromoneRegistry.sol";

contract PheromoneRegistryTest is Test {
    PheromoneRegistry registry;

    function setUp() public {
        registry = new PheromoneRegistry();
    }

    // --- deposit() tests ---

    /// @dev Happy path: deposit a THREAT pheromone.
    function test_deposit_happyPath() public { /* ... */ }

    /// @dev Revert: vector not 1280 bytes.
    function test_deposit_invalidVector() public { /* ... */ }

    /// @dev Revert: intensity below MIN_INTENSITY.
    function test_deposit_intensityTooLow() public { /* ... */ }

    /// @dev Revert: intensity above MAX_INTENSITY.
    function test_deposit_intensityTooHigh() public { /* ... */ }

    /// @dev Revert: insufficient stake.
    function test_deposit_insufficientStake() public { /* ... */ }

    /// @dev Verify: event emitted with correct params.
    function test_deposit_emitsEvent() public { /* ... */ }

    // --- currentIntensity() decay tests ---

    /// @dev At deposit block: intensity equals initial.
    function test_intensity_atDeposit() public { /* ... */ }

    /// @dev At 1 half-life: intensity ~= initial / 2.
    function test_intensity_oneHalfLife() public { /* ... */ }

    /// @dev At 2 half-lives: intensity ~= initial / 4.
    function test_intensity_twoHalfLives() public { /* ... */ }

    /// @dev At 10 half-lives: intensity ~= 0 (dead).
    function test_intensity_tenHalfLives() public { /* ... */ }

    /// @dev At 20+ half-lives: returns exactly 0.
    function test_intensity_twentyHalfLives() public { /* ... */ }

    /// @dev Different pheromone types have different decay rates.
    function test_intensity_typeSpecificDecay() public { /* ... */ }

    // --- confirm() alpha paradox tests ---

    /// @dev Confirmation reduces effective half-life.
    function test_confirm_reducesHalfLife() public { /* ... */ }

    /// @dev 2 confirmations: hl = base / 3.
    function test_confirm_twoConfs() public { /* ... */ }

    /// @dev Confirmed pheromone decays faster than unconfirmed.
    function test_confirm_fasterDecay() public { /* ... */ }

    /// @dev Revert: confirm a dead pheromone.
    function test_confirm_deadReverts() public { /* ... */ }

    /// @dev Revert: double-confirm.
    function test_confirm_doubleReverts() public { /* ... */ }

    /// @dev Confirm preserves current intensity (not original).
    function test_confirm_preservesCurrentIntensity() public { /* ... */ }

    // --- SINR tests ---

    /// @dev Single pheromone: SINR = intensity * SCALE / NOISE_FLOOR.
    function test_sinr_singlePheromone() public { /* ... */ }

    /// @dev Two identical pheromones: SINR ~ SCALE / (intensity + NOISE).
    function test_sinr_twoIdentical() public { /* ... */ }

    /// @dev 10 identical pheromones: SINR drops significantly.
    function test_sinr_tenIdentical() public { /* ... */ }

    /// @dev Different types do not interfere.
    function test_sinr_differentTypesNoInterference() public { /* ... */ }

    /// @dev Dead pheromones do not contribute to interference.
    function test_sinr_deadNotCounted() public { /* ... */ }

    // --- cleanup() tests ---

    /// @dev Happy path: cleanup a dead pheromone.
    function test_cleanup_deadPheromone() public { /* ... */ }

    /// @dev Revert: cleanup an alive pheromone.
    function test_cleanup_aliveReverts() public { /* ... */ }

    /// @dev Verify: storage cleared after cleanup.
    function test_cleanup_clearsStorage() public { /* ... */ }

    /// @dev Verify: removed from location index.
    function test_cleanup_removedFromLocationIndex() public { /* ... */ }

    /// @dev Batch cleanup processes all dead, skips alive.
    function test_cleanupBatch_mixedAliveAndDead() public { /* ... */ }

    // --- Fixed-point decay accuracy tests ---

    /// @dev Decay at r=hl/2 is within 6% of true value (0.707).
    function test_decayAccuracy_midpoint() public { /* ... */ }

    /// @dev Decay at r=0 is exact (returns integerDecay).
    function test_decayAccuracy_exactHalving() public { /* ... */ }

    /// @dev Decay at r=hl is exact (returns integerDecay / 2).
    function test_decayAccuracy_exactBoundary() public { /* ... */ }
}
```

---

## 8. Anti-Pattern Checklist

These are known bugs and design mistakes specific to these contracts. The
implementing agent MUST verify that none of these anti-patterns exist in
the final code.

### InsightBoard Anti-Patterns

| # | Anti-Pattern | Why It Is Wrong | Correct Pattern |
|---|-------------|-----------------|-----------------|
| 1 | `require(state == State.ACTIVE)` in `confirm()` | Blocks SUBMITTED->ACTIVE transition, DECAYING->ACTIVE recovery, and CHALLENGED->ACTIVE resolution. | Accept 4 states: SUBMITTED, ACTIVE, DECAYING, CHALLENGED. |
| 2 | `block.number - publishBlock` for age in `computeState()` | Re-confirmation does not reset the decay clock. An insight confirmed 1 block ago still appears to be decaying if publishBlock was long ago. | Use `block.number - lastConfirmedBlock`. |
| 3 | Missing `publishBlock != 0` existence check | Non-existent insights have state == 0 (SUBMITTED) and pass the state check. Confirming writes phantom state to storage. | Always check `insight.anchor.publishBlock != 0` before operating. |
| 4 | Emitting events before state changes | Violates CEI (Checks-Effects-Interactions). If the state change reverts after the event, the event log is inconsistent. | Update all storage, THEN emit events. |
| 5 | Using `msg.sender` for `author` without documenting proxy behavior | If deployed behind a delegatecall proxy, `msg.sender` is the end-user. If behind a forwarding proxy (call), `msg.sender` is the proxy. Mixing these up breaks authorship. | Document the proxy pattern. If using delegatecall, `msg.sender` is correct. If forwarding, consider `tx.origin` or an explicit parameter (with access control). |
| 6 | `confirmsSinceChallenge` not reset on challenge | If not reset, residual confirmations from a previous challenge carry over, making it easier to resolve the new challenge. | Reset `confirmsSinceChallenge = 0` whenever entering CHALLENGED state. |
| 7 | Tier demotion in `confirm()` | If the tier promotion logic uses `else if` chains without `<` guards, a PERSISTENT insight that drops below 25 confirmations due to a code bug could be demoted. | Tier promotions are monotonic. Use `insight.anchor.tier < uint8(Tier.X)` guards. |

### PheromoneRegistry Anti-Patterns

| # | Anti-Pattern | Why It Is Wrong | Correct Pattern |
|---|-------------|-----------------|-----------------|
| 1 | Using `float` or `f64` for decay on-chain | Non-deterministic across validators. `2.0f64.powf(-x)` can produce different results on different platforms. | Use fixed-point integer arithmetic with bit shifts for the integer part and linear interpolation for the fractional part. |
| 2 | Storing decayed intensity (write-on-read) | Requires SSTORE on every read, wastes gas, creates unnecessary state changes. | Compute decay at read time from `(intensity, depositBlock)`. Store only the initial parameters. |
| 3 | Not resetting `depositBlock` on confirmation | After alpha paradox reduces the half-life, the old `depositBlock` means the age is computed from the original deposit, not from the confirmation. The pheromone appears much older than it should under the new half-life. | Reset `depositBlock = block.number` and `intensity = currentIntensity()` on confirmation. |
| 4 | Simple summation instead of SINR | An attacker deposits 100 pheromones at one location to create a 100x signal. No defense against signal flooding. | Use SINR: `target * SCALE / (sum_interferers + NOISE_FLOOR)`. |
| 5 | Alpha paradox inverted (confirmation extends half-life) | Contradicts the information-theoretic design. Widely-known signals should fade faster, not persist. | `new_hl = base_hl / (1 + n_confs)`. |
| 6 | Division by zero in SINR when no interferers and no noise floor | `intensity / 0` reverts in Solidity. | Always add `NOISE_FLOOR` (= 10) to the denominator. |
| 7 | Unbounded `_locationPheromones` array | Gas DoS on `sinr()` if an attacker fills a location with thousands of deposits. | Add a per-location cap or paginate the SINR computation. |

### Cross-Contract Anti-Patterns

| # | Anti-Pattern | Why It Is Wrong | Correct Pattern |
|---|-------------|-----------------|-----------------|
| 1 | Using `block.timestamp` for time-based logic | `block.timestamp` can be manipulated by validators within the 15-second window. Block number is deterministic. | Use `block.number` for all time-based computations. |
| 2 | Relying on `tx.origin` for authentication | Breaks composability with other contracts and is susceptible to phishing attacks. | Use `msg.sender`. |
| 3 | External calls before state updates (CEI violation) | Reentrancy risk. A malicious contract called before state is finalized can re-enter and exploit stale state. | Always: Checks -> Effects -> Interactions. |
| 4 | Using `transfer()` or `send()` for ETH transfers | Hard-coded 2300 gas stipend breaks with EIP-1884. | Use `call{value: amount}("")` and check the return value. |

---

## Audit Findings

**Audit Date:** 2026-05-08
**Auditor:** Automated spec-vs-implementation diff
**Files Audited:**

| File | Path |
|------|------|
| Spec | `/Users/will/dev/nunchi/daeji/tmp/HDC/impl/19-solidity-specs.md` |
| HdcPrecompile.sol | `/Users/will/dev/nunchi/daeji/contracts/src/HdcPrecompile.sol` |
| IInsightBoard.sol | `/Users/will/dev/nunchi/daeji/contracts/src/IInsightBoard.sol` |
| IPheromoneRegistry.sol | `/Users/will/dev/nunchi/daeji/contracts/src/IPheromoneRegistry.sol` |
| InsightBoard.sol | `/Users/will/dev/nunchi/daeji/contracts/src/InsightBoard.sol` |
| PheromoneRegistry.sol | `/Users/will/dev/nunchi/daeji/contracts/src/PheromoneRegistry.sol` |
| InsightBoard.t.sol | `/Users/will/dev/nunchi/daeji/contracts/test/InsightBoard.t.sol` |
| PheromoneRegistry.t.sol | `/Users/will/dev/nunchi/daeji/contracts/test/PheromoneRegistry.t.sol` |
| foundry.toml | `/Users/will/dev/nunchi/daeji/contracts/foundry.toml` |

### F01: Function Signatures -- All Match

| Contract | Function | Spec Signature | Impl Signature | Status |
|----------|----------|----------------|----------------|--------|
| HdcLib | `hamming` | `hamming(bytes,bytes) -> uint32` | `hamming(bytes,bytes) -> uint32` | MATCH |
| HdcLib | `bind` | `bind(bytes,bytes) -> bytes` | `bind(bytes,bytes) -> bytes` | MATCH |
| HdcLib | `bundle` | `bundle(bytes[]) -> bytes` | `bundle(bytes[]) -> bytes` | MATCH |
| HdcLib | `permute` | `permute(bytes,uint32) -> bytes` | `permute(bytes,uint32) -> bytes` | MATCH |
| HdcLib | `storeVector` | `storeVector(bytes32,bytes)` | `storeVector(bytes32,bytes)` | MATCH |
| HdcLib | `searchSimilar` | `searchSimilar(bytes,uint8) -> (bytes32[],uint16[])` | `searchSimilar(bytes,uint8) -> (bytes32[],uint16[])` | MATCH |
| HdcLib | `deleteVector` | `deleteVector(bytes32)` | `deleteVector(bytes32)` | MATCH |
| IInsightBoard | `submit` | `submit(Kind,bytes,bytes) payable -> bytes32` | `submit(Kind,bytes,bytes) payable -> bytes32` | MATCH |
| IInsightBoard | `confirm` | `confirm(bytes32)` | `confirm(bytes32)` | MATCH |
| IInsightBoard | `challenge` | `challenge(bytes32,bytes32)` | `challenge(bytes32,bytes32)` | MATCH |
| IInsightBoard | `renew` | `renew(bytes32) payable` | `renew(bytes32) payable` | MATCH |
| IInsightBoard | `purge` | `purge(bytes32)` | `purge(bytes32)` | MATCH |
| IInsightBoard | `computeState` | `computeState(bytes32) -> State` | `computeState(bytes32) -> State` | MATCH |
| IInsightBoard | `searchSimilar` | `searchSimilar(bytes,uint8) -> (bytes32[],uint16[])` | `searchSimilar(bytes,uint8) -> (bytes32[],uint16[])` | MATCH |
| IPheromoneRegistry | `deposit` | `deposit(bytes,PheromoneType,uint64) payable` | `deposit(bytes,PheromoneType,uint64) payable` | MATCH |
| IPheromoneRegistry | `confirm` | `confirm(bytes32)` | `confirm(bytes32)` | MATCH |
| IPheromoneRegistry | `cleanup` | `cleanup(bytes32)` | `cleanup(bytes32)` | MATCH |
| IPheromoneRegistry | `cleanupBatch` | `cleanupBatch(bytes32[])` | `cleanupBatch(bytes32[])` | MATCH |
| IPheromoneRegistry | `currentIntensity` | `currentIntensity(bytes32) -> uint64` | `currentIntensity(bytes32) -> uint64` | MATCH |
| IPheromoneRegistry | `sinr` | `sinr(bytes32) -> uint64` | `sinr(bytes32) -> uint64` | MATCH |
| IPheromoneRegistry | `readPheromones` | `readPheromones(bytes,PheromoneType,uint8) -> (bytes32[],uint64[])` | `readPheromones(bytes,PheromoneType,uint8) -> (bytes32[],uint64[])` | MATCH |

### F02: Event Signatures -- All Match

| Contract | Event | Spec Signature | Impl Signature | Status |
|----------|-------|----------------|----------------|--------|
| IInsightBoard | `InsightPublished` | `(bytes32 indexed, bytes32 indexed, address indexed, bytes, bytes, uint8, uint8)` | Same | MATCH |
| IInsightBoard | `InsightConfirmed` | `(bytes32 indexed, address indexed, uint64)` | Same | MATCH |
| IInsightBoard | `InsightChallenged` | `(bytes32 indexed, bytes32 indexed, address indexed)` | Same | MATCH |
| IInsightBoard | `InsightStateChanged` | `(bytes32 indexed, uint8, uint8)` | Same | MATCH |
| IInsightBoard | `InsightRenewed` | `(bytes32 indexed, address indexed)` | Same | MATCH |
| IInsightBoard | `InsightPurged` | `(bytes32 indexed, address indexed)` | Same | MATCH |
| IPheromoneRegistry | `PheromoneDeposited` | `(bytes32 indexed, bytes32 indexed, address indexed, uint8, uint64, uint64)` | Same | MATCH |
| IPheromoneRegistry | `PheromoneConfirmed` | `(bytes32 indexed, address indexed, uint64, uint64)` | Same | MATCH |
| IPheromoneRegistry | `PheromonePruned` | `(bytes32 indexed, address indexed)` | Same | MATCH |

### F03: Struct Layouts -- All Match

| Contract | Struct | Spec Layout | Impl Layout | Status |
|----------|--------|-------------|-------------|--------|
| InsightBoard | `InsightAnchor` | `vectorHash(bytes32), contentHash(bytes32), author(address), publishBlock(uint64), kind(uint8), tier(uint8), state(uint8)` -- 3 slots | Same field order and types | MATCH |
| InsightBoard | `Insight` | `anchor(InsightAnchor), confirmations(uint64), lastConfirmedBlock(uint64), confirmsSinceChallenge(uint64), stakedAmount(uint256)` -- 5 slots total | Same field order and types | MATCH |
| PheromoneRegistry | `PheromoneDeposit` | `locationHash(bytes32), depositor(address), intensity(uint64), depositBlock(uint64), confirmationCount(uint16), pType(uint8)` -- 3 slots | Same field order and types | MATCH |

### F04: Enum Layouts -- All Match

| Contract | Enum | Spec Values | Impl Values | Status |
|----------|------|-------------|-------------|--------|
| IInsightBoard | `Kind` | INSIGHT(0), HEURISTIC(1), ANTI_KNOWLEDGE(2), WARNING(3), CAUSAL_LINK(4), STRATEGY(5) | Same | MATCH |
| IInsightBoard | `State` | SUBMITTED(0), VERIFIED(1), ACTIVE(2), CHALLENGED(3), DECAYING(4), ARCHIVED(5), PURGED(6) | Same | MATCH |
| IInsightBoard | `Tier` | TRANSIENT(0), WORKING(1), CONSOLIDATED(2), PERSISTENT(3) | Same | MATCH |
| IPheromoneRegistry | `PheromoneType` | THREAT(0), OPPORTUNITY(1), WISDOM(2) | Same | MATCH |

### F05: Constants -- All Match

| Contract | Constant | Spec Value | Impl Value | Status |
|----------|----------|------------|------------|--------|
| InsightBoard | `DUPLICATE_THRESHOLD` | 512 | 512 | MATCH |
| InsightBoard | `MIN_STAKE` | 0.01 ether | 0.01 ether | MATCH |
| InsightBoard | `CHALLENGE_RESOLUTION_CONFS` | 5 | 5 | MATCH |
| InsightBoard | `RESONANCE_THRESHOLD` | 1024 | 1024 | MATCH |
| PheromoneRegistry | `MIN_PHEROMONE_STAKE` | 0.001 ether | 0.001 ether | MATCH |
| PheromoneRegistry | `MIN_INTENSITY` | 100 | 100 | MATCH |
| PheromoneRegistry | `MAX_INTENSITY` | 10,000 | 10,000 | MATCH |
| PheromoneRegistry | `DEATH_THRESHOLD` | 1 | 1 | MATCH |
| PheromoneRegistry | `NOISE_FLOOR` | 10 | 10 | MATCH |
| PheromoneRegistry | `SINR_SCALE` | 10,000 | 10,000 | MATCH |
| PheromoneRegistry | `PROXIMITY_THRESHOLD` | 1024 | 1024 | MATCH |
| PheromoneRegistry | `FP_SCALE` | 65536 | 65536 | MATCH |

### F06: Logic Deviations from Spec

#### F06-1: `PheromoneRegistry.confirm()` -- Intensity Snapshot Ordering (DEVIATION)

**Spec** (lines 1265-1273 of this document): The spec snapshots `currentIntensity` AFTER incrementing `confirmationCount`:
```
_confirmers[pheromoneId][msg.sender] = true;
p.confirmationCount++;
// The reduced half-life takes effect at the CURRENT block.
uint64 currentInt = _computeIntensity(p);
p.intensity = currentInt;
p.depositBlock = uint64(block.number);
```

**Implementation** (`PheromoneRegistry.sol`, lines 149-159): The implementation snapshots intensity BEFORE incrementing `confirmationCount`:
```solidity
_confirmers[pheromoneId][msg.sender] = true;
// Snapshot current intensity BEFORE changing half-life
uint64 currentInt = _computeIntensity(p);
p.confirmationCount++;
// Reset depositBlock so decay restarts from now with shorter half-life.
p.intensity = currentInt;
p.depositBlock = uint64(block.number);
```

**Impact:** In the spec, `_computeIntensity` runs with the ALREADY-incremented `confirmationCount`, meaning the snapshot uses the NEW (shorter) half-life to compute the intensity. In the implementation, the snapshot uses the OLD (longer) half-life. The implementation approach is arguably more correct because the intent is to preserve the current actual intensity as of this moment, before the half-life change takes effect. The spec's ordering would cause a small intensity jump (up or down depending on age). **This is a beneficial deviation but should be reconciled in the spec.**

**Severity:** Low. The implementation is more correct than the spec.

#### F06-2: `PheromoneRegistry.confirm()` -- Confirmation count increment placement

Directly related to F06-1. The spec increments `confirmationCount` before computing intensity; the implementation increments after. The order matters because `_computeIntensity` calls `_effectiveHalfLife`, which uses `confirmationCount`.

#### F06-3: `PheromoneRegistry._computeIntensity()` -- Fractional Decay Formula Difference

**Spec** (lines 1413-1416): Uses FP_SCALE-based formula:
```
2^(-r/hl) ~ (FP_SCALE - (FP_SCALE/2 * r / hl)) / FP_SCALE
```

**Implementation** (`PheromoneRegistry.sol`, lines 307-312): Uses simpler direct formula:
```solidity
uint64 numerator = 2 * hl - r;
uint64 denominator = 2 * hl;
uint128 result = (uint128(integerDecay) * uint128(numerator)) / uint128(denominator);
```

**Impact:** Both formulas are algebraically equivalent: `(2*hl - r) / (2*hl) = 1 - r/(2*hl)`, and `(FP_SCALE - FP_SCALE/2 * r/hl) / FP_SCALE = 1 - r/(2*hl)`. The implementation avoids the FP_SCALE constant entirely, producing identical results with fewer operations. The constant `FP_SCALE = 65536` is declared but never used in the implementation.

**Severity:** None (functionally equivalent). However, the unused `FP_SCALE` constant wastes contract bytecode size.

### F07: Missing Implementations from Spec

#### F07-1: `readPheromones()` is a Stub

**File:** `/Users/will/dev/nunchi/daeji/contracts/src/PheromoneRegistry.sol`, lines 254-263

The function returns empty arrays. The spec acknowledges this as a placeholder (spec lines 1376-1388), but it means this interface function is not usable.

```solidity
function readPheromones(...) external view returns (bytes32[] memory ids, uint64[] memory sinrValues) {
    ids = new bytes32[](0);
    sinrValues = new uint64[](0);
}
```

#### F07-2: Missing Deployment Script

**Spec section 6** defines a `DeployHDC` Foundry script in `script/DeployHDC.s.sol`. No such file exists in the contracts directory.

#### F07-3: Missing `HdcLib.t.sol` Test File

**Spec section 7** defines a test file structure including `test/HdcLib.t.sol` and `test/mocks/MockHdcPrecompile.sol`. Neither exists.

#### F07-4: Missing Test Scenarios from Spec

The spec lists the following test functions that have no implementation:

**InsightBoard.t.sol:**
| Spec Test | Status |
|-----------|--------|
| `test_submit_duplicateRejected` | MISSING (precompile not available in tests) |
| `test_submit_idIncludesSender` | MISSING |
| `test_confirm_decayingToActive` | MISSING |
| `test_confirm_archivedReverts` | MISSING |
| `test_confirm_purgedReverts` | MISSING |
| `test_confirm_tierNeverDemotes` | MISSING |
| `test_challenge_emitsEvents` | MISSING |
| `test_computeState_kindHalfLives` | MISSING |
| `test_renew_archivedToActive` | MISSING (only `test_renew_nonArchivedReverts` exists) |
| `test_renew_insufficientStakeReverts` | MISSING |
| `test_renew_resetsBlocks` | MISSING |
| `test_purge_deletesFromPrecompile` | MISSING (precompile not testable) |
| `test_purge_reentrancySafe` | MISSING |

**PheromoneRegistry.t.sol:**
| Spec Test | Status |
|-----------|--------|
| `test_confirm_fasterDecay` | MISSING |
| `test_sinr_tenIdentical` | MISSING |
| `test_sinr_deadNotCounted` | MISSING |
| `test_cleanup_removedFromLocationIndex` | MISSING |
| `test_decayAccuracy_exactBoundary` | MISSING |

---

## Implementation Status

| Component | Status | Notes |
|-----------|--------|-------|
| **HdcPrecompile.sol (HdcLib)** | COMPLETE | Exact match with spec. All 7 functions implemented. |
| **IInsightBoard.sol** | COMPLETE | Exact match with spec. All enums, events, and function signatures match. |
| **IPheromoneRegistry.sol** | COMPLETE | Exact match with spec. All enums, events, and function signatures match. |
| **InsightBoard.sol** | COMPLETE | All functions implemented. Logic matches spec. CEI pattern followed. |
| **PheromoneRegistry.sol** | 95% | All core functions work. `readPheromones()` is a stub returning empty arrays. Minor confirm() ordering deviation (beneficial). |
| **DeployHDC.s.sol** | NOT STARTED | Deployment script defined in spec but not created. |
| **InsightBoard.t.sol** | 70% | 23 tests implemented; 13 spec-listed tests missing. TestableInsightBoard subclass used to bypass precompile. |
| **PheromoneRegistry.t.sol** | 80% | 20 tests implemented; 5 spec-listed tests missing. |
| **HdcLib.t.sol** | NOT STARTED | No tests for the library exist. Requires MockHdcPrecompile. |
| **foundry.toml** | COMPLETE | Properly configured: `solc_version = "0.8.28"`, `evm_version = "cancun"`, `via_ir = true`, `optimizer = true` with 200 runs. |

### Anti-Pattern Checklist Verification

All 7 InsightBoard anti-patterns from spec section 8 have been avoided:

| # | Anti-Pattern | Avoided? | Evidence |
|---|-------------|----------|----------|
| 1 | `require(state == ACTIVE)` in confirm() | YES | `InsightBoard.sol` lines 155-161: accepts SUBMITTED, ACTIVE, DECAYING, CHALLENGED |
| 2 | `block.number - publishBlock` for age | YES | `InsightBoard.sol` line 310: `uint64(block.number) - insight.lastConfirmedBlock` |
| 3 | Missing `publishBlock != 0` existence check | YES | Present in `confirm()` (line 149), `challenge()` (lines 205, 209), `renew()` (line 241), `purge()` (line 267) |
| 4 | Events before state changes | YES | All functions emit events after storage writes |
| 5 | Undocumented proxy behavior | PARTIAL | No proxy-related documentation exists, but contracts are not designed as proxy-compatible either. Acceptable for v1. |
| 6 | `confirmsSinceChallenge` not reset on challenge | YES | `InsightBoard.sol` line 229: `target.confirmsSinceChallenge = 0` |
| 7 | Tier demotion | YES | `InsightBoard.sol` lines 376-382: uses `< uint8(Tier.X)` guards; promotions are monotonic |

All 7 PheromoneRegistry anti-patterns from spec section 8 have been avoided:

| # | Anti-Pattern | Avoided? | Evidence |
|---|-------------|----------|----------|
| 1 | Float/f64 for decay | YES | All integer arithmetic with bit shifts and linear interpolation |
| 2 | Storing decayed intensity (write-on-read) | YES | `_computeIntensity()` computes at read time from stored `(intensity, depositBlock)` |
| 3 | Not resetting depositBlock on confirmation | YES | `PheromoneRegistry.sol` line 159: `p.depositBlock = uint64(block.number)` |
| 4 | Simple summation instead of SINR | YES | `sinr()` function uses full SINR formula with NOISE_FLOOR |
| 5 | Alpha paradox inverted | YES | `_effectiveHalfLife()` line 274: `base / (1 + confirmationCount)` -- divides, not multiplies |
| 6 | Division by zero in SINR | YES | `NOISE_FLOOR = 10` always added to denominator (line 249) |
| 7 | Unbounded `_locationPheromones` | PRESENT | No per-location cap exists. See Security Concerns S03. |

All 4 cross-contract anti-patterns from spec section 8 have been avoided:

| # | Anti-Pattern | Avoided? | Evidence |
|---|-------------|----------|----------|
| 1 | Using `block.timestamp` | YES | All time logic uses `block.number` |
| 2 | Relying on `tx.origin` | YES | Only `msg.sender` is used |
| 3 | CEI violation | YES | All functions follow Checks-Effects-Interactions |
| 4 | Using `transfer()`/`send()` | YES | `InsightBoard.purge()` line 290: uses `author.call{value: legacy}("")` |

---

## Anti-Patterns & Duct Tape

### D01: TestableInsightBoard Duplicates Core Logic

**File:** `/Users/will/dev/nunchi/daeji/contracts/test/InsightBoard.t.sol`, lines 11-90

The `TestableInsightBoard` contract is a subclass that overrides `submit()` and `purge()` to skip precompile calls. This is duct tape: the entire function body is copy-pasted from `InsightBoard.sol` with precompile lines removed. If the base contract changes, the test subclass will silently drift.

**Better approach:** Use `vm.etch()` to deploy a mock precompile at `address(0x09)` that returns valid responses. This eliminates code duplication and tests the actual contract code path, including precompile interaction encoding.

### D02: Unused Constant `FP_SCALE`

**File:** `/Users/will/dev/nunchi/daeji/contracts/src/PheromoneRegistry.sol`, line 39

```solidity
uint64 internal constant FP_SCALE = 65536;
```

This constant is declared but never referenced anywhere in the implementation. The `_computeIntensity()` function uses the simpler `2*hl` formula instead. Dead code increases bytecode size.

### D03: No Custom Errors

Both contracts use string-based `require()` messages throughout (e.g., `"InsightBoard: invalid vector size"`). Since the target is Solidity ^0.8.20, custom errors (`error InsufficientStake()`) are available and save ~200+ bytes of deployment bytecode plus gas on revert paths.

### D04: No Access-Control on `purge()` Remaining ETH

**File:** `/Users/will/dev/nunchi/daeji/contracts/src/InsightBoard.sol`, lines 277-293

The `purge()` function sends 10% of stake to the original author but the remaining 90% is permanently locked in the contract. There is no mechanism to withdraw this ETH. Over time, the contract will accumulate stranded funds.

### D05: No `receive()` or `fallback()` Function

**File:** `/Users/will/dev/nunchi/daeji/contracts/src/InsightBoard.sol`

The `InsightBoard` contract receives ETH via `submit()` and `renew()` (both `payable`), but has no `receive()` function. If ETH is sent directly (not via a payable function), it will revert. This is acceptable behavior but should be documented. More importantly, the contract accumulates ETH from purge (90% remains) with no withdrawal mechanism.

### D06: `PheromoneRegistry.deposit()` Does Not Store Location Vector in Precompile

**File:** `/Users/will/dev/nunchi/daeji/contracts/src/PheromoneRegistry.sol`, lines 84-131

Unlike `InsightBoard.submit()` which calls `HdcLib.storeVector()`, the `PheromoneRegistry.deposit()` function does not store the location vector in the precompile index. This means `readPheromones()` cannot use precompile search to find nearby pheromones, which is why F07-1 is a stub. The spec acknowledges this gap (spec lines 1376-1388) but it means the pheromone spatial query functionality is entirely non-operational.

### D07: Hardcoded `topK = 5` in Duplicate Check

**File:** `/Users/will/dev/nunchi/daeji/contracts/src/InsightBoard.sol`, line 80

```solidity
HdcLib.searchSimilar(vector, 5);
```

The `topK` for duplicate detection is hardcoded to 5. If the index contains more than 5 near-duplicate vectors at varying distances, only the 5 closest are checked. A near-duplicate at position 6 would bypass the check. This should be a configurable constant.

---

## Security Concerns

### S01: Reentrancy in `purge()` -- Mitigated but Fragile

**File:** `/Users/will/dev/nunchi/daeji/contracts/src/InsightBoard.sol`, lines 263-293

The `purge()` function deletes storage before the ETH transfer (line 283 `delete insights[insightId]` before line 290 `author.call{value: legacy}`), which follows CEI. However, the function does NOT have a `nonReentrant` modifier. The protection relies entirely on the `delete` causing re-entry to fail the existence check. This is correct but fragile -- if any future refactor moves the `delete` after the transfer, reentrancy becomes exploitable. A `nonReentrant` modifier from OpenZeppelin would provide defense-in-depth.

### S02: Front-Running of `confirm()` in InsightBoard and PheromoneRegistry

Both `confirm()` functions are permissionless and have no cooldown. An attacker observing a pending confirmation transaction could front-run it. For `InsightBoard`, this is mostly harmless (the original confirmer can still confirm). For `PheromoneRegistry`, front-running a confirmation accelerates decay (alpha paradox), which could be weaponized: an attacker Sybil-confirms a competitor's pheromone to kill it faster. The spec notes this (spec line 1536) and suggests a per-pheromone rate limit. No rate limit is implemented.

### S03: Unbounded `_locationPheromones` Array -- Gas DoS

**File:** `/Users/will/dev/nunchi/daeji/contracts/src/PheromoneRegistry.sol`, lines 62, 120, 232

The `_locationPheromones[locationHash]` array grows without bound. `sinr()` iterates this entire array (line 232). An attacker can deposit thousands of pheromones at one locationHash (cost: 0.001 ETH each), making `sinr()` calls for that location consume millions of gas and effectively DoS the view function. The spec identifies this (spec lines 1527-1532) and recommends a per-location cap. No cap is implemented.

### S04: No Rate Limiting on `deposit()` or `submit()`

Both contracts accept unlimited deposits from the same address in the same block. The only protection is the minimum stake. On a chain with low native-token value, an attacker could flood the system cheaply.

### S05: Stake Accumulation Without Withdrawal

**File:** `/Users/will/dev/nunchi/daeji/contracts/src/InsightBoard.sol`

`submit()` and `renew()` accept ETH stakes. `purge()` returns only 10% to the original author. The remaining 90% stays in the contract forever. Over time, this creates a growing pool of locked ETH with no governance mechanism, owner, or withdrawal function. The PheromoneRegistry has the same issue: `deposit()` accepts ETH (0.001 ether minimum) but there is zero mechanism to ever withdraw it.

### S06: `renew()` Checks Stored State, Not Computed State

**File:** `/Users/will/dev/nunchi/daeji/contracts/src/InsightBoard.sol`, lines 244-245

```solidity
require(
    insight.anchor.state == uint8(State.ARCHIVED),
    "InsightBoard: only ARCHIVED insights can be renewed"
);
```

This checks the *stored* state, not `computeState()`. An insight that is ACTIVE in storage but whose computed state is ARCHIVED (due to age > 5x half-life) cannot be renewed because the stored state is still ACTIVE. The user would need to wait until the stored state matches, but there is no transaction that updates stored state to ARCHIVED -- it only changes via `computeState()` which is a view function. This means insights that decay through ARCHIVED to PURGED without any intermediate transaction can never be renewed; they can only be purged.

### S07: `confirm()` in InsightBoard Does Not Check `computeState()`

**File:** `/Users/will/dev/nunchi/daeji/contracts/src/InsightBoard.sol`, lines 154-161

The `confirm()` function checks the *stored* state, not `computeState()`. An insight whose stored state is ACTIVE but whose computed state is ARCHIVED (no confirmations for >5x half-life) can still be confirmed, transitioning it back to ACTIVE. This is arguably a feature (re-confirmation recovery), but it means the ARCHIVED and PURGED computed states can be "overridden" by a single confirmation. The spec's state diagram shows DECAYING->ACTIVE on re-confirm, but does not show ARCHIVED->ACTIVE via confirm (only via `renew()`).

---

## Recommended Changes Checklist

### Priority 1 -- Must Fix

- [ ] **S06**: Change `renew()` to check `computeState(insightId) == State.ARCHIVED` instead of checking stored state directly. Without this, insights that decay to ARCHIVED without an intermediate transaction cannot be renewed.
  - File: `/Users/will/dev/nunchi/daeji/contracts/src/InsightBoard.sol`, line 245
  - Change: `insight.anchor.state == uint8(State.ARCHIVED)` to `computeState(insightId) == State.ARCHIVED`

- [ ] **S07**: In `confirm()`, add a check against computed ARCHIVED/PURGED states to prevent confirming insights that have decayed past their confirmable lifecycle. The stored state may be stale.
  - File: `/Users/will/dev/nunchi/daeji/contracts/src/InsightBoard.sol`, after line 153
  - Add: `State computed = computeState(insightId); require(computed != State.ARCHIVED && computed != State.PURGED, "InsightBoard: insight has decayed");`

- [ ] **S05**: Add a mechanism for reclaiming stranded ETH. Options: (a) send remaining 90% to a protocol treasury address, (b) return full stake to author on purge, (c) add an owner-controlled `withdraw()`. For PheromoneRegistry, add a withdrawal mechanism or return stakes on cleanup.

### Priority 2 -- Should Fix

- [ ] **S03/D06**: Implement a per-location cap on `_locationPheromones` (e.g., max 100 per locationHash). Reject deposits that would exceed the cap.
  - File: `/Users/will/dev/nunchi/daeji/contracts/src/PheromoneRegistry.sol`, after line 120

- [ ] **S01**: Add `ReentrancyGuard` from OpenZeppelin to `InsightBoard` and apply `nonReentrant` to `purge()`.
  - File: `/Users/will/dev/nunchi/daeji/contracts/src/InsightBoard.sol`

- [ ] **D01**: Replace `TestableInsightBoard` with a mock precompile deployed via `vm.etch(address(0x09), ...)`. This eliminates code duplication in tests and exercises the actual contract code.
  - File: `/Users/will/dev/nunchi/daeji/contracts/test/InsightBoard.t.sol`

- [ ] **D03**: Replace string revert messages with custom errors across all contracts. Saves ~2KB+ deployment bytecode and reduces revert gas cost.
  - Files: All `.sol` files in `contracts/src/`

- [ ] **F06-1**: Update spec section 5 to match the implementation's confirm() ordering (snapshot intensity before incrementing confirmationCount). The implementation is more correct.

### Priority 3 -- Nice to Have

- [ ] **D02**: Remove the unused `FP_SCALE` constant from `PheromoneRegistry.sol` line 39.

- [ ] **D07**: Extract `topK = 5` in duplicate check to a named constant (e.g., `uint8 public constant DUPLICATE_CHECK_TOP_K = 5`).
  - File: `/Users/will/dev/nunchi/daeji/contracts/src/InsightBoard.sol`, line 80

- [ ] **F07-1**: Implement `readPheromones()` or document it as explicitly unsupported in v1 by removing it from the interface.

- [ ] **F07-2**: Create `script/DeployHDC.s.sol` per spec section 6.

- [ ] **F07-3**: Create `test/HdcLib.t.sol` and `test/mocks/MockHdcPrecompile.sol`.

- [ ] **F07-4**: Implement the 18 missing test scenarios listed in F07-4 above.

- [ ] **Gas**: Consider using `unchecked` blocks for loop increments (`i++`) in `PheromoneRegistry.sinr()`, `cleanupBatch()`, `_removeFromLocationIndex()`, and `InsightBoard.submit()` duplicate check loop. Solidity 0.8+ overflow checks on loop counters bounded by array length are redundant.

- [ ] **Gas**: Consider caching `atLocation.length` in a local variable in `PheromoneRegistry.sinr()` to avoid repeated SLOAD on each loop iteration (line 232).

- [ ] **Foundry Config**: The `foundry.toml` specifies `solc_version = "0.8.28"` while the pragma is `^0.8.20`. Both are compatible, but the pragma could be tightened to `^0.8.28` or `=0.8.28` for reproducible builds.

---

## Second-Pass Remediation Detail

**Audit date:** 2026-05-08
**Scope read:** `contracts/src/HdcPrecompile.sol`, `contracts/src/IInsightBoard.sol`,
`contracts/src/InsightBoard.sol`, `contracts/src/IPheromoneRegistry.sol`,
`contracts/src/PheromoneRegistry.sol`, `contracts/test/InsightBoard.t.sol`,
`contracts/test/PheromoneRegistry.t.sol`, `contracts/script/DeployHDC.s.sol`,
and `contracts/foundry.toml`.

**External references verified:** Solidity documents Checks-Effects-Interactions
as checks first, storage effects second, and external interactions last, and also
documents the withdrawal pattern as the recommended way to decouple effects from
Ether delivery:
https://docs.soliditylang.org/en/v0.8.30/security-considerations.html#use-the-checks-effects-interactions-pattern
and
https://docs.soliditylang.org/en/v0.8.30/common-patterns.html#withdrawal-from-contracts.
OpenZeppelin Contracts v5 places `ReentrancyGuard` under
`@openzeppelin/contracts/utils/ReentrancyGuard.sol`:
https://docs.openzeppelin.com/contracts/5.x/api/utils#ReentrancyGuard.

### R01: HdcPrecompile.sol Library and Precompile Contract

**Problem:** The checked-in `HdcPrecompile.sol` dispatch table does not match
the Rust v1 precompile: Solidity currently uses stateful `store/search/delete`
opcodes and `0x07` for `permute`, while Rust uses stateless `0x01=hamming`,
`0x02=bind`, `0x03=bundle`, `0x04=permute`, `0x05=vectorId`,
`0x06=isSimilar`. `bundle()` also repeatedly calls `abi.encodePacked` inside a
loop, which reallocates and copies the whole payload each iteration. Return
decoding trusts precompile output shape, and stateful search accepts `topK = 0`
even though no matching Rust opcode exists.

**Concrete fix:**

- Update the Solidity library to the Rust v1 table:
  `0x01=hamming`, `0x02=bind`, `0x03=bundle`, `0x04=permute`,
  `0x05=vectorId`, `0x06=isSimilar`. Do not document `0x07=permute` for v1.
  If stored-vector challenge checks are kept ABI-stable, add a future v2 opcode
  such as `0x08 = hammingStored(bytes32 idA, bytes32 idB) -> uint16` and make
  it consensus-backed.
- Add constants in the library/spec: `VECTOR_BYTES = 1280`,
  `MAX_TOP_K = 50`, and `MAX_BUNDLE_VECTORS = 32` unless the Rust gas schedule
  proves a higher bound is safe.
- Replace string reverts with custom errors:
  `InvalidVectorSize(uint256 actual)`, `InvalidTopK(uint256 topK)`,
  `TooManyVectors(uint256 count)`, `HdcPrecompileCallFailed(uint8 selector)`,
  and `InvalidHdcReturn(uint8 selector, uint256 actualLength)`.
- In `bundle()`, allocate one bytes buffer of length `1 + 4 + n * VECTOR_BYTES`
  and copy vector chunks into it once. Do not append with `abi.encodePacked` in
  the loop.
- In `searchSimilar()`, require `topK > 0 && topK <= MAX_TOP_K`, decode the
  return, then require `ids.length == distances.length && ids.length <= topK`.
- Never use `HDC_PRECOMPILE.code.length` as an availability check. EVM
  precompiles normally have no bytecode, so deployment and tests should probe
  the selector behavior instead.

### R02: InsightBoard State Materialization

**Problem:** `computeState()` is view-only and can return `DECAYING`,
`ARCHIVED`, or `PURGED` while `anchor.state` remains `ACTIVE` or `CHALLENGED`.
Mutating functions currently check the stored state in several places. This
means `renew()` cannot renew an insight that is computed `ARCHIVED` unless
storage was already materialized, and `confirm()` can revive computed
`ARCHIVED` or `PURGED` insights because it sees the stale stored `ACTIVE` state.

**Concrete fix:**

- Add an internal `_computedState(Insight storage insight) view returns (State)`
  helper that contains the current `computeState()` math without doing an extra
  mapping lookup.
- Add `_materializeState(bytes32 id, Insight storage insight) internal returns
  (State oldState, State currentState)`:
  - compute the current state;
  - if the computed state differs from `anchor.state`, write it to
    `anchor.state`;
  - emit `InsightStateChanged(id, oldState, uint8(currentState))` exactly once
    for this automatic transition.
- Use `_materializeState()` at the start of `confirm()`, `challenge()`,
  `renew()`, and `purge()` after the existence check.
- `confirm()` should allow only `SUBMITTED`, `ACTIVE`, `DECAYING`, and
  `CHALLENGED` after materialization. It should reject materialized
  `ARCHIVED` with `InsightArchived(insightId)` and `PURGED` with
  `InsightPurgedState(insightId)`.
- `renew()` should require materialized `ARCHIVED`, not stored `ARCHIVED`.
- `challenge()` should require the materialized target state to be `ACTIVE` and
  the materialized anti-knowledge state to be non-terminal. If the resonance
  check remains part of the design, keep the `challenge(bytes32,bytes32)` ABI
  and add a stored-vector HDC precompile selector rather than storing full
  vectors in contract storage.

### R03: InsightBoard Stake Accounting and Withdrawals

**Problem:** `submit()` and `renew()` accept native-token stake. `purge()` pays
only 10% to the author and leaves the rest permanently trapped. The direct
author `call` can also make purge fail if the author is a contract that rejects
ETH. CEI is currently mostly followed, but the direct transfer couples cleanup
to recipient behavior.

**Concrete fix:**

- Add explicit accounting:
  `uint256 public totalInsightStake;`,
  `mapping(address => uint256) public pendingWithdrawals;`, and
  `uint256 public treasuryAccrued;`.
- On `submit()` and `renew()`, increment `totalInsightStake` by `msg.value` and
  store the per-insight `stakedAmount` as today.
- On `purge()`, cache `stakedAmount`, delete/materialize state, remove from the
  HDC index, then account the full stake without transferring:
  - `authorRefund = stakedAmount / 10`;
  - `purgeBounty = stakedAmount / 100` if a caller incentive is desired;
  - `treasuryShare = stakedAmount - authorRefund - purgeBounty`;
  - `pendingWithdrawals[author] += authorRefund`;
  - `pendingWithdrawals[msg.sender] += purgeBounty`;
  - `treasuryAccrued += treasuryShare`;
  - `totalInsightStake -= stakedAmount`.
- Add `withdraw()` or `claim()` that reads `pendingWithdrawals[msg.sender]`,
  sets it to zero before the external call, and then uses
  `call{value: amount}("")`. This implements the Solidity withdrawal pattern
  and avoids blocking `purge()` on a rejecting recipient.
- Add `sweepTreasury(address payable treasury)` gated by governance/owner or
  make the treasury address immutable in the constructor. Do not leave an
  unowned ETH balance with no withdrawal path.
- Import and inherit OpenZeppelin v5 `ReentrancyGuard` in `InsightBoard`:
  `import {ReentrancyGuard} from "@openzeppelin/contracts/utils/ReentrancyGuard.sol";`
  then mark `withdraw()` and any treasury sweep function `nonReentrant`.
  `purge()` no longer needs to transfer ETH, but `nonReentrant` is acceptable
  defense-in-depth if it continues to touch external precompile state.

### R04: PheromoneRegistry Indexing, Stake, and Cleanup

**Problem:** `deposit()` does not store the location vector in the HDC
precompile, so `readPheromones()` cannot work and currently returns empty
arrays. Deposited stake is not stored per pheromone and has no withdrawal or
treasury path. `_locationPheromones` has unbounded growth, `sinr()` loops over
the entire array, and `_removeFromLocationIndex()` is O(n) despite its comment
claiming O(1).

**Concrete fix:**

- Extend `PheromoneDeposit` with `uint256 stakedAmount` unless the design is
  changed to require `msg.value == MIN_PHEROMONE_STAKE`. Tracking the actual
  value is safer because callers can currently overpay.
- Add `totalPheromoneStake`, `pendingWithdrawals`, and `treasuryAccrued`
  accounting mirroring the InsightBoard withdrawal model.
- On `deposit()`, call `HdcLib.storeVector(indexId, location)` after state is
  written. Use a namespaced index key:
  `indexId = keccak256(abi.encodePacked("PHEROMONE", pheromoneId))`, and store
  `mapping(bytes32 => bytes32) public pheromoneIdByIndexId`.
- On `cleanup()` and `cleanupBatch()`, delete the namespaced HDC vector with
  `HdcLib.deleteVector(indexId)` after caching all accounting values.
- Implement `readPheromones()`:
  - require valid vector length and bounded `topK`;
  - search the HDC precompile with the query vector;
  - map namespaced index IDs back to pheromone IDs;
  - filter missing, dead, wrong-type, and distance `> PROXIMITY_THRESHOLD`;
  - return at most `topK` pheromone IDs and their current `sinr()` values.
- Replace `_removeFromLocationIndex()` with a real O(1) swap-and-pop index:
  `mapping(bytes32 => mapping(bytes32 => uint256)) locationIndexPlusOne`.
  Update the moved element's index when swapping.
- Add `MAX_PHEROMONES_PER_LOCATION = 100`, `MAX_CLEANUP_BATCH = 50`, and
  `MAX_READ_TOP_K = 50`. `deposit()` should reject a location at cap unless the
  caller first cleans dead entries.
- Use `uint256 sumInterferers` in `sinr()` and only downcast at return. With a
  per-location cap, overflow becomes structurally bounded.
- Make `_effectiveHalfLife()` return at least `1` block or cap confirmations.
  Today, `base / (1 + confirmationCount)` reaches zero for highly confirmed
  THREAT pheromones, which makes intensity immediately zero.
- Remove `FP_SCALE` if the direct `(2 * hl - r) / (2 * hl)` formula remains.

### R05: Custom Errors and Event Surface

**Problem:** All contracts use string-based `require()` messages. This increases
deployment bytecode and makes tests brittle. Events are mostly present, but
stake and withdrawal accounting would be invisible without new events. The
current `InsightPublished` event includes the full vector and arbitrary content,
which is necessary only if logs are the index-rebuild source; otherwise it is an
unbounded log-bloat surface.

**Concrete fix:**

- Define custom errors in the relevant interfaces or concrete contracts:
  `InvalidVectorSize(uint256 actual)`, `InsufficientStake(uint256 sent,uint256 min)`,
  `InsightDoesNotExist(bytes32 id)`, `InvalidInsightState(bytes32 id,uint8 state)`,
  `AlreadyConfirmed(bytes32 id,address confirmer)`, `TooSimilar(bytes32 id,uint16 distance)`,
  `InvalidPheromoneType(uint8 pType)`, `PheromoneDead(bytes32 id)`,
  `LocationCapacityExceeded(bytes32 locationHash)`, `BatchTooLarge(uint256 size)`,
  and `WithdrawalFailed(address account,uint256 amount)`.
- Update Foundry tests to use selector-based `vm.expectRevert` instead of
  string matching.
- Add events for accounting:
  `StakeAccounted(bytes32 indexed id, address indexed account, uint256 amount, uint8 action)`,
  `WithdrawalCredited(address indexed account, uint256 amount, bytes32 indexed sourceId)`,
  `WithdrawalClaimed(address indexed account, uint256 amount)`, and
  `TreasuryAccrued(bytes32 indexed sourceId, uint256 amount)`.
- Emit `InsightStateChanged` for automatic materialization transitions before
  the explicit transition event for the user action. Tests should assert that a
  stale `ACTIVE -> ARCHIVED -> ACTIVE` renew path emits both transitions in
  order.
- Add a `MAX_CONTENT_BYTES` cap or replace raw `content` in
  `InsightPublished` with `contentHash` plus a bounded URI. If the precompile
  index is rebuilt from logs, keep the full 1,280-byte vector event but cap all
  other dynamic payloads.

### R06: Gas and DoS Bounds

**Problem:** The current implementation has several unbounded paths:
`content` can be arbitrarily large, `bundle()` has no count bound,
`cleanupBatch()` has no length bound, `_locationPheromones` has no cap, and
`sinr()` can become unusable at a spammed location. Solidity's own security
guidance warns that loops depending on storage values can exceed the block gas
limit and stall contract paths.

**Concrete fix:**

- Add protocol constants and enforce them at entry points:
  `MAX_CONTENT_BYTES`, `MAX_TOP_K`, `MAX_BUNDLE_VECTORS`,
  `MAX_PHEROMONES_PER_LOCATION`, and `MAX_CLEANUP_BATCH`.
- Cache array lengths in loops and use `unchecked { ++i; }` only after bounds
  checks are in place.
- Make gas snapshots part of Foundry CI:
  `forge snapshot` for `submit`, `confirm`, `purge`, `deposit`, `sinr` at 1,
  10, and cap-sized location arrays, and `cleanupBatch(MAX_CLEANUP_BATCH)`.
- Document exact precompile gas costs for `storeVector`, `deleteVector`,
  `searchSimilar`, `bundle`, `permute`, and any new stored-vector selector.
  Solidity-side bounds should match the Rust precompile gas schedule.

### R07: Foundry Test Remediation

**Problem:** `InsightBoard.t.sol` uses `TestableInsightBoard`, which copies
production logic and skips HDC calls. This can pass while production
`InsightBoard` fails. There is no `HdcLib.t.sol` or mock precompile coverage.
Current tests do not cover stale computed state, withdrawal accounting,
DoS bounds, or deployment smoke checks.

**Concrete fix:**

- Replace `TestableInsightBoard` with tests against the real `InsightBoard`.
  Use `vm.mockCall`/`vm.expectCall` for simple HDC interactions and add a
  raw-fallback `MockHdcPrecompile` etched at `address(0x09)` for integration
  cases. The mock must parse the first payload byte, not ABI selectors.
- Add `HdcLib.t.sol` for:
  invalid vector sizes, invalid `topK`, invalid return lengths, selector bytes
  for all operations, `bundle()` count cap, `permute()` selector `0x07`, and
  stored-vector selector if added.
- Add InsightBoard tests for:
  computed `ARCHIVED` can be renewed, computed `ARCHIVED/PURGED` cannot be
  confirmed, `DECAYING` can be confirmed back to `ACTIVE`, materialization
  emits events once, reentrant/reverting author cannot block purge, withdrawal
  zeroes credit before `call`, stake totals always reconcile, duplicate checks
  respect HDC distances, and `challenge()` enforces anti-knowledge resonance.
- Add PheromoneRegistry tests for:
  `deposit()` stores a namespaced vector, `readPheromones()` returns filtered
  IDs and SINR values, wrong types and far distances are filtered, dead entries
  do not interfere, location cap rejection, O(1) index removal after swap,
  batch cap rejection, half-life minimum behavior, and stake withdrawal
  reconciliation.
- Add deployment tests for `DeployHDC.s.sol`:
  missing `DEPLOYER_PRIVATE_KEY` fails, missing/zero `TREASURY_ADDRESS` fails
  if treasury is introduced, HDC precompile smoke probe fails fast on chains
  without selector support, and successful deployment logs both addresses.

### R08: Deployment and Upgrade Implications

**Problem:** The current `contracts/script/DeployHDC.s.sol` exists and deploys
both contracts with no constructor args. The first-pass note saying the script
is missing is now stale. The remediation above likely introduces constructor
parameters, an OpenZeppelin dependency, and possibly ABI/storage changes.

**Concrete fix:**

- Install and pin OpenZeppelin Contracts v5 for Foundry, then add the remapping
  required by the installed layout, normally:
  `@openzeppelin/contracts/=lib/openzeppelin-contracts/contracts/`.
- Update `DeployHDC.s.sol` to read `TREASURY_ADDRESS`, validate it is nonzero,
  pass it to both contracts, and log the deployed version/config constants.
- Add a deploy-time HDC precompile probe before broadcasting user-facing
  addresses. Probe `hamming` with two zero 1,280-byte vectors and, if required,
  `permute` and `hammingStored` selector support.
- Treat adding `stakedAmount` to `PheromoneDeposit`, adding accounting storage,
  and inheriting `ReentrancyGuard` as storage-layout changes. They are safe for
  fresh deployments but require a migration plan if any proxy or live address
  already exists.
- If no proxy is intended, document that these are immutable v1 deployments. If
  a proxy is introduced later, use upgradeable OpenZeppelin variants and add
  storage-gap/layout tests before deployment.
- Publish deployment artifacts with the precompile selector table and gas
  schedule version. The contracts and the Rust precompile must be upgraded in
  lockstep when selector `0x07` documentation or `0x08` stored-vector support
  changes.

---

## 9. PR #42 Alignment Notes (Added 2026-05-08)

### 9.1 HdcLib.sol Opcode Mismatch

The current `HdcLib` library uses raw 1-byte opcode dispatch to address `0x09`.
PR #42's `kora-precompiles` crate uses 4-byte function selectors to address
`0xA0C`. The opcode assignments are completely incompatible:

| Operation | HdcLib.sol Opcode | Rust (this branch) | Rust (PR #42) |
|-----------|-------------------|---------------------|---------------|
| hamming | `0x06` | `0x01` | `keccak("hammingDistance(bytes,bytes)")[0:4]` |
| bind | `0x05` | `0x02` | `keccak("bind(bytes,bytes)")[0:4]` |
| bundle | `0x04` | `0x03` | `keccak("bundle(bytes[])")[0:4]` |
| permute | `0x07` | `0x04` | `keccak("permute(bytes,uint256)")[0:4]` |
| storeVector | `0x01` | N/A | N/A |
| searchSimilar | `0x02` | N/A | `keccak("search(bytes,uint256)")[0:4]` |
| deleteVector | `0x03` | N/A | N/A |
| vector_id | N/A | `0x05` | N/A |
| is_similar | N/A | `0x06` | N/A |
| projectBytes | N/A | N/A | `keccak("projectBytes(bytes)")[0:4]` |
| projectTokens | N/A | N/A | `keccak("projectTokens(string)")[0:4]` |

Every single `HdcLib` call sends a different opcode than Rust expects. The
Solidity contract tests hide this because `TestableInsightBoard` bypasses
the precompile entirely.

### 9.2 PheromoneRegistry.sol Supersession

PR #42 implements stigmergy as a second Rust precompile at address `0xA0D`
(StigmergyPrecompile). This supersedes `PheromoneRegistry.sol` for the
following reasons:

1. **Performance:** Rust precompile avoids SSTORE gas overhead for pheromone
   tracking. Decay computation in Rust is ~100x faster than EVM integer
   emulation.
2. **Consensus safety:** The Rust implementation uses `BTreeMap` for
   deterministic iteration. The Solidity implementation's
   `_locationPheromones` array manipulation is O(n) for removal.
3. **Alpha paradox:** PR #42 implements the alpha paradox (confirmation reduces
   half-life) directly in Rust with fixed-point integer arithmetic.

**Decision needed:** Delete `PheromoneRegistry.sol` or keep it as a fallback
for chains that do not support the `0xA0D` precompile.

### 9.3 InsightBoard.sol Status

`InsightBoard.sol` remains active but requires updates:

1. It calls `HdcLib.searchSimilar()` in `submit()` -- this must be updated to
   use the PR #42 selector for `search(bytes,uint256)`.
2. It calls `HdcLib.storeVector()` in `submit()` -- this operation may not
   exist in PR #42 (search index is event-driven, not precompile-stored).
3. It calls `HdcLib.deleteVector()` in `purge()` -- same issue.
4. The precompile address must change from `0x09` to `0xA0C`.

---

## 10. Reconciliation Checklist (PR #42 Alignment)

- [ ] **Reconcile HdcLib.sol selector assignments with kora-precompiles.**
  Replace raw opcode dispatch with 4-byte function selectors matching
  PR #42's Rust precompile. Update `HDC_PRECOMPILE` address from `0x09` to
  `0xA0C`.

- [ ] **Update InsightBoard.sol to use 4-byte selectors for precompile calls.**
  All `abi.encodePacked(uint8(opcode), ...)` patterns must become standard
  ABI-encoded calls via the new `IHDCPrecompile` interface.

- [ ] **Decide fate of PheromoneRegistry.sol.**
  Options: (a) Delete entirely, rely on StigmergyPrecompile at `0xA0D`.
  (b) Keep as a Solidity fallback for non-precompile chains. (c) Convert to
  a thin wrapper that calls the Stigmergy precompile.

- [ ] **Add IHDCPrecompile.sol interface matching kora-precompiles.**
  Define a Solidity interface with all PR #42 function signatures. This
  interface can be used by `InsightBoard.sol` and any other contract that
  needs HDC vector operations.

  ```solidity
  // SPDX-License-Identifier: MIT
  pragma solidity ^0.8.20;

  interface IHDCPrecompile {
      function projectBytes(bytes calldata data) external view returns (bytes memory);
      function projectTokens(string calldata text) external view returns (bytes memory);
      function search(bytes calldata query, uint256 topK) external view returns (bytes32[] memory ids, uint256[] memory distances);
      function hammingDistance(bytes calldata a, bytes calldata b) external view returns (uint256);
      function bind(bytes calldata a, bytes calldata b) external view returns (bytes memory);
      function bundle(bytes[] calldata vectors) external view returns (bytes memory);
      function permute(bytes calldata v, uint256 positions) external view returns (bytes memory);
  }
  ```

- [ ] **Add IStigmergyPrecompile.sol interface.**

  ```solidity
  // SPDX-License-Identifier: MIT
  pragma solidity ^0.8.20;

  interface IStigmergyPrecompile {
      function deposit(bytes calldata location, uint8 pType, uint64 intensity) external returns (bytes32);
      function readPheromones(bytes calldata query, uint8 pType, uint8 topK) external view returns (bytes32[] memory, uint64[] memory);
      function currentIntensity(bytes32 pheromoneId) external view returns (uint64);
      function confirm(bytes32 pheromoneId) external;
      function cleanup(bytes32 pheromoneId) external;
  }
  ```

- [ ] **Update test contracts.** Replace `TestableInsightBoard` with tests
  that use `vm.mockCall()` against the correct precompile address and
  selector format. Add integration tests that exercise the real precompile.

- [ ] **Update deployment script.** `DeployHDC.s.sol` must account for the
  precompile address change and verify precompile availability at `0xA0C`
  and `0xA0D` before deploying contracts.
