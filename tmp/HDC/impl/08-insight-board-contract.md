# Implementation Guide: InsightBoard.sol

> **STATUS: SPEC COMPLETE, IMPLEMENTATION HAS CRITICAL BUGS**
>
> The InsightBoard contract is implemented and deployed with all core functions
> (`submit`, `confirm`, `challenge`, `computeState`, `renew`, `purge`). The
> implementation diverges from this spec in positive ways (richer events, hash-based
> IDs, anti-knowledge gating on challenge) and in negative ways (stored-vs-computed
> state inconsistency makes `renew()` unreachable for naturally-decayed insights,
> resonance check is declared but never enforced, test mocks duplicate core logic).
>
> **Audit:** 2026-05-08 -- 16 findings (F01-F16), 5 anti-patterns (AP01-AP05),
> 5 security concerns (SEC01-SEC05). See Audit Findings section below.
>
> **PR #42 note:** PR #42 treats InsightBoard as an external contract (event
> decoder only). The contract itself lives in the `contracts-core` repo at
> `contracts/src/InsightBoard.sol`. PR #42's precompile uses event-replay to
> rebuild the vector index from `InsightPublished` events.
>
> **Last updated:** 2026-05-08

**Contract:** `InsightBoard.sol` -- 7-state FSM for on-chain knowledge lifecycle
**Target:** Solidity ^0.8.20, deployed on the daeji chain
**Depends on:** HDC precompile at address `0x09` (IHdcPrecompile interface)

This document gives you everything you need to implement `InsightBoard.sol` from
scratch. No prior context is assumed. Read top to bottom.

---

## What This Contract Does

The InsightBoard is the on-chain governance layer for a shared knowledge
substrate. Agents publish "insights" (knowledge entries) by submitting a hash
of an HDC hypervector and a content hash. Each insight moves through a 7-state
lifecycle -- from submission through active use, decay, and eventual removal.
The contract tracks confirmations (independent validations by other agents),
manages tier promotions (TRANSIENT -> WORKING -> CONSOLIDATED -> PERSISTENT),
and computes time-based state transitions using block numbers.

All arithmetic is integer. There is no floating point anywhere in this contract.

---

## State Transition Diagram

```
                                confirm()
                           (any of these states)
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
              |         |    |  +-----+------+
              |         |    |        |
              |         |    v        |
              |         | CHALLENGED -+-----> ARCHIVED
              |         |    |                (no confs for
              |         |    |                 2x half-life)
              |         |    |
              |         |    +-----> ACTIVE
              |         |           (5+ new confs
              |         |            since challenge)
              |         |
              |    age > 1x         age > 5x          age > 10x
              |    half-life        half-life          half-life
              +----> DECAYING -----> ARCHIVED -------> PURGED


Transitions summary:
  SUBMITTED   --[1st confirm()]--> ACTIVE
  SUBMITTED   --[age > 1x HL]---> DECAYING  (via computeState)
  ACTIVE      --[confirm()]------> ACTIVE    (refresh: resets lastConfirmedBlock)
  ACTIVE      --[challenge()]----> CHALLENGED
  ACTIVE      --[age > 1x HL]---> DECAYING  (via computeState)
  CHALLENGED  --[5+ confs]-------> ACTIVE
  CHALLENGED  --[age > 2x HL]---> ARCHIVED  (via computeState)
  DECAYING    --[confirm()]------> ACTIVE    (recovery path)
  DECAYING    --[age > 5x HL]---> ARCHIVED  (via computeState)
  ARCHIVED    --[renew()]--------> ACTIVE
  ARCHIVED    --[age > 10x HL]--> PURGED    (via computeState)
  PURGED      --[purge()]--------> (removed from storage)

Where "HL" = effective half-life = kindHalfLife(kind) * tierMultiplier(tier)
Where "age" = block.number - lastConfirmedBlock  (NOT publishBlock)
```

---

## Enums and Constants

```solidity
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.20;

/// @notice Interface for the HDC precompile at address 0x09.
/// The precompile maintains an in-memory vector index rebuilt from events.
interface IHdcPrecompile {
    function storeVector(bytes32 id, bytes calldata vector) external;
    function deleteVector(bytes32 id) external;
    function searchSimilar(bytes calldata query, uint8 topK)
        external view returns (bytes32[] memory ids, uint16[] memory distances);
}
```

**Knowledge kinds (6 types):**

```solidity
enum Kind {
    INSIGHT,         // 0 -- General insight (72h base half-life)
    HEURISTIC,       // 1 -- Reusable heuristic (168h)
    ANTI_KNOWLEDGE,  // 2 -- Contradiction of existing knowledge (336h)
    WARNING,         // 3 -- Urgent warning (48h)
    CAUSAL_LINK,     // 4 -- Cause-effect relationship (240h)
    STRATEGY         // 5 -- Strategic pattern (120h)
}
```

**Knowledge tiers (4 levels) -- promotions happen automatically via confirm():**

```solidity
enum Tier {
    TRANSIENT,     // 0 -- Default on submission (1x multiplier)
    WORKING,       // 1 -- Promoted at 3 confirmations (3x multiplier)
    CONSOLIDATED,  // 2 -- Promoted at 10 confirmations (7x multiplier)
    PERSISTENT     // 3 -- Promoted at 25 confirmations (10x multiplier)
}
```

**Lifecycle states (7 states):**

```solidity
enum State {
    SUBMITTED,   // 0 -- Just published, awaiting first confirmation
    VERIFIED,    // 1 -- Reserved for future verification pipelines (not used currently)
    ACTIVE,      // 2 -- Confirmed and participating in search results
    CHALLENGED,  // 3 -- Anti-knowledge published against this entry
    DECAYING,    // 4 -- Age exceeded 1x effective half-life
    ARCHIVED,    // 5 -- Age exceeded 5x effective half-life (or 2x for CHALLENGED)
    PURGED       // 6 -- Age exceeded 10x effective half-life, eligible for removal
}
```

---

## Storage Layout

### InsightAnchor (95 bytes, 3 SSTORE slots)

This is the permanent on-chain record for each insight. Solidity packs the
third slot automatically because `address` (20 bytes) + `uint64` (8 bytes) +
`uint8` + `uint8` + `uint8` = 31 bytes, which fits in one 32-byte slot.

```solidity
struct InsightAnchor {
    bytes32 vectorHash;    // slot 1: keccak256 of the 1,280-byte HDC vector
    bytes32 contentHash;   // slot 2: keccak256 of the knowledge content
    address author;        // slot 3 (20 bytes) --|
    uint64  publishBlock;  //         (8 bytes)  --| 31 bytes total, packed
    uint8   kind;          //         (1 byte)   --|
    uint8   tier;          //         (1 byte)   --|
    uint8   state;         //         (1 byte)   --|
}
```

### Insight (full struct, 5 SSTORE slots total)

```solidity
struct Insight {
    InsightAnchor anchor;           // 3 slots (vectorHash, contentHash, packed)
    uint64 confirmations;           // --|
    uint64 lastConfirmedBlock;      //   |-- slot 4 (packed: 8+8+8 = 24 bytes)
    uint64 confirmsSinceChallenge;  // --|
    uint256 stakedAmount;           // slot 5
}
```

### Other storage

```solidity
mapping(bytes32 => Insight) public insights;
mapping(bytes32 => mapping(address => bool)) public confirmers;
```

The `confirmers` mapping prevents double-confirmation by the same address.

### Constants

```solidity
uint16 constant DUPLICATE_THRESHOLD = 512;   // Hamming distance for near-duplicate detection
uint256 constant MIN_STAKE = 0.01 ether;     // Minimum stake per insight
IHdcPrecompile constant HDC_PRECOMPILE = IHdcPrecompile(address(0x09));
```

---

## Events

```solidity
event InsightPublished(
    uint256 indexed id,
    address indexed author,
    bytes32 vectorHash,
    uint8 kind
);

event InsightConfirmed(
    uint256 indexed id,
    address indexed confirmer,
    uint32 confirmations
);

event InsightChallenged(
    uint256 indexed id,
    address indexed challenger
);

event InsightStateChanged(
    uint256 indexed id,
    State oldState,
    State newState
);
```

> **Note:** The design also defines a richer `InsightPublished` event that
> includes the full 1,280-byte vector and content bytes in non-indexed fields
> (for event-log reconstruction of the precompile index). If your deployment
> needs that, add `bytes vector` and `bytes content` as non-indexed parameters
> to `InsightPublished`. This adds ~13,350 gas per submission.

---

## Full Contract Implementation

```solidity
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.20;

/// @notice Interface for the HDC precompile deployed at address 0x09.
interface IHdcPrecompile {
    function storeVector(bytes32 id, bytes calldata vector) external;
    function deleteVector(bytes32 id) external;
    function searchSimilar(bytes calldata query, uint8 topK)
        external view returns (bytes32[] memory ids, uint16[] memory distances);
}

/// @title InsightBoard
/// @notice 7-state FSM governing the lifecycle of on-chain knowledge entries.
///         Each insight is a hash anchor (vectorHash + contentHash) with
///         time-based decay, community confirmation, and tier promotion.
contract InsightBoard {
    // -----------------------------------------------------------------------
    // Types
    // -----------------------------------------------------------------------

    enum Kind { INSIGHT, HEURISTIC, ANTI_KNOWLEDGE, WARNING, CAUSAL_LINK, STRATEGY }
    enum State { SUBMITTED, VERIFIED, ACTIVE, CHALLENGED, DECAYING, ARCHIVED, PURGED }
    enum Tier { TRANSIENT, WORKING, CONSOLIDATED, PERSISTENT }

    struct InsightAnchor {
        bytes32 vectorHash;    // keccak256 of the 1,280-byte HDC vector
        bytes32 contentHash;   // keccak256 of the knowledge content
        address author;        // msg.sender at submission
        uint64  publishBlock;  // block.number at submission
        uint8   kind;          // Kind enum (0-5)
        uint8   tier;          // Tier enum (0-3)
        uint8   state;         // State enum (0-6)
    }

    struct Insight {
        InsightAnchor anchor;
        uint64 confirmations;
        uint64 lastConfirmedBlock;
        uint64 confirmsSinceChallenge;
        uint256 stakedAmount;
    }

    // -----------------------------------------------------------------------
    // Constants
    // -----------------------------------------------------------------------

    uint16 constant DUPLICATE_THRESHOLD = 512;
    uint256 constant MIN_STAKE = 0.01 ether;
    IHdcPrecompile constant HDC_PRECOMPILE = IHdcPrecompile(address(0x09));

    // -----------------------------------------------------------------------
    // Storage
    // -----------------------------------------------------------------------

    /// @dev Auto-incrementing counter for insight IDs.
    uint256 private _nextId;

    /// @dev Insight ID -> Insight data.
    mapping(uint256 => Insight) public insights;

    /// @dev Insight ID -> confirmer address -> has confirmed.
    mapping(uint256 => mapping(address => bool)) public confirmers;

    // -----------------------------------------------------------------------
    // Events
    // -----------------------------------------------------------------------

    event InsightPublished(
        uint256 indexed id,
        address indexed author,
        bytes32 vectorHash,
        uint8 kind
    );

    event InsightConfirmed(
        uint256 indexed id,
        address indexed confirmer,
        uint32 confirmations
    );

    event InsightChallenged(
        uint256 indexed id,
        address indexed challenger
    );

    event InsightStateChanged(
        uint256 indexed id,
        State oldState,
        State newState
    );

    // -----------------------------------------------------------------------
    // submit()
    // -----------------------------------------------------------------------

    /// @notice Publish a new insight to the shared substrate.
    /// @param vectorHash keccak256 hash of the full 1,280-byte HDC vector.
    /// @param contentHash keccak256 hash of the knowledge content.
    /// @param kind The KnowledgeKind (0-5). See Kind enum.
    /// @return insightId The ID assigned to this insight.
    ///
    /// Gas breakdown:
    ///   Base tx cost:                      21,000
    ///   Calldata (two bytes32 + uint8):    ~2,200
    ///   3x cold SSTORE (new slots):       66,300  (22,100 each)
    ///   Event (InsightPublished, LOG2):    ~1,100
    ///   -----------------------------------------------
    ///   Total:                            ~88,400
    ///
    /// If you also call the HDC precompile for duplicate checking and vector
    /// storage, add ~50,000 gas for the precompile interaction.
    function submit(
        bytes32 vectorHash,
        bytes32 contentHash,
        uint8 kind
    ) external payable returns (uint256 insightId) {
        require(kind <= uint8(Kind.STRATEGY), "Invalid kind");
        require(msg.value >= MIN_STAKE, "Insufficient stake");

        insightId = _nextId++;

        insights[insightId] = Insight({
            anchor: InsightAnchor({
                vectorHash: vectorHash,
                contentHash: contentHash,
                author: msg.sender,
                publishBlock: uint64(block.number),
                kind: kind,
                tier: uint8(Tier.TRANSIENT),
                state: uint8(State.SUBMITTED)
            }),
            confirmations: 0,
            lastConfirmedBlock: uint64(block.number),
            confirmsSinceChallenge: 0,
            stakedAmount: msg.value
        });

        emit InsightPublished(insightId, msg.sender, vectorHash, kind);
    }

    // -----------------------------------------------------------------------
    // confirm()
    // -----------------------------------------------------------------------

    /// @notice Confirm an existing insight. Increments the confirmation count,
    ///         resets the decay clock (lastConfirmedBlock), and triggers tier
    ///         promotion at 3 / 10 / 25 confirmations.
    /// @param insightId The ID of the insight to confirm.
    ///
    /// CRITICAL: This function MUST accept SUBMITTED, ACTIVE, DECAYING, and
    /// CHALLENGED states. A prior audit found a bug where only ACTIVE was
    /// accepted, breaking the DECAYING->ACTIVE recovery path and the
    /// CHALLENGED->ACTIVE resolution path.
    ///
    /// State transitions on confirmation:
    ///   SUBMITTED   -> ACTIVE   (first confirmation)
    ///   ACTIVE      -> ACTIVE   (refreshes lastConfirmedBlock)
    ///   DECAYING    -> ACTIVE   (recovery: revives a neglected insight)
    ///   CHALLENGED  -> ACTIVE   (resolution: 5+ new confs since challenge)
    ///
    /// Gas breakdown:
    ///   Cold SLOAD (insight):              2,100
    ///   Cold SSTORE (confirmers mapping): 22,100  (zero-to-nonzero)
    ///   Warm SSTORE (confirmations):       2,900  (nonzero-to-nonzero)
    ///   Warm SSTORE (lastConfirmedBlock):  2,900
    ///   Warm SSTORE (state, if changed):   2,900  (conditional)
    ///   Warm SSTORE (tier, if promoted):   2,900  (conditional)
    ///   Event (InsightConfirmed, LOG2):    ~1,100
    ///   -----------------------------------------------
    ///   Total:                            ~28,000 (no state/tier change)
    ///                                     ~33,800 (with state + tier change)
    function confirm(uint256 insightId) external {
        Insight storage insight = insights[insightId];

        // --- Existence check ---
        // A non-existent insight has publishBlock == 0 and state == 0
        // (SUBMITTED). Without this guard, confirming a non-existent ID
        // writes phantom state to storage.
        require(insight.anchor.publishBlock != 0, "Insight does not exist");

        // --- Double-confirmation check ---
        require(!confirmers[insightId][msg.sender], "Already confirmed");

        // --- State check ---
        // MUST accept all four confirmable states. Do NOT restrict to ACTIVE only.
        uint8 currentState = insight.anchor.state;
        require(
            currentState == uint8(State.SUBMITTED) ||
            currentState == uint8(State.ACTIVE) ||
            currentState == uint8(State.DECAYING) ||
            currentState == uint8(State.CHALLENGED),
            "Not confirmable"
        );

        // --- Record confirmation ---
        confirmers[insightId][msg.sender] = true;
        insight.confirmations++;
        insight.lastConfirmedBlock = uint64(block.number);

        // --- State transitions ---
        if (currentState == uint8(State.SUBMITTED) ||
            currentState == uint8(State.DECAYING)) {
            // SUBMITTED -> ACTIVE on first confirmation.
            // DECAYING  -> ACTIVE on re-confirmation (recovery path).
            _setState(insightId, insight, State.ACTIVE);
            insight.confirmsSinceChallenge = 0;
        } else if (currentState == uint8(State.CHALLENGED)) {
            // CHALLENGED -> ACTIVE requires 5+ NEW confirmations since the
            // challenge was issued. We track these separately from the
            // lifetime confirmation count.
            insight.confirmsSinceChallenge++;
            if (insight.confirmsSinceChallenge >= 5) {
                _setState(insightId, insight, State.ACTIVE);
                insight.confirmsSinceChallenge = 0;
            }
        }
        // If ACTIVE, no state change -- just refresh lastConfirmedBlock.

        // --- Tier promotion ---
        // Thresholds: 3 -> WORKING, 10 -> CONSOLIDATED, 25 -> PERSISTENT.
        // Check in descending order so a single call that crosses multiple
        // thresholds (e.g., from 2 to 3 confirmations) lands at the right tier.
        uint64 confs = insight.confirmations;
        if (confs >= 25 && insight.anchor.tier < uint8(Tier.PERSISTENT)) {
            insight.anchor.tier = uint8(Tier.PERSISTENT);
        } else if (confs >= 10 && insight.anchor.tier < uint8(Tier.CONSOLIDATED)) {
            insight.anchor.tier = uint8(Tier.CONSOLIDATED);
        } else if (confs >= 3 && insight.anchor.tier < uint8(Tier.WORKING)) {
            insight.anchor.tier = uint8(Tier.WORKING);
        }

        emit InsightConfirmed(
            insightId,
            msg.sender,
            uint32(insight.confirmations)
        );
    }

    // -----------------------------------------------------------------------
    // challenge()
    // -----------------------------------------------------------------------

    /// @notice Move an ACTIVE insight to CHALLENGED state.
    /// @param insightId The ID of the insight to challenge.
    ///
    /// In the full system, challenges are triggered when anti-knowledge is
    /// published whose vector has HDC similarity > 0.7 with the target. This
    /// function is the on-chain state transition. The similarity check can be
    /// done off-chain or via the precompile before calling this function.
    ///
    /// Gas breakdown:
    ///   Cold SLOAD (insight):            2,100
    ///   Warm SSTORE (state):             2,900
    ///   Event (InsightChallenged, LOG2): ~1,100
    ///   Event (InsightStateChanged):     ~1,100
    ///   -----------------------------------------------
    ///   Total:                           ~8,000
    function challenge(uint256 insightId) external {
        Insight storage insight = insights[insightId];
        require(insight.anchor.publishBlock != 0, "Insight does not exist");
        require(
            insight.anchor.state == uint8(State.ACTIVE),
            "Only ACTIVE insights can be challenged"
        );

        _setState(insightId, insight, State.CHALLENGED);
        insight.confirmsSinceChallenge = 0;

        emit InsightChallenged(insightId, msg.sender);
    }

    // -----------------------------------------------------------------------
    // computeState()
    // -----------------------------------------------------------------------

    /// @notice Compute the current lifecycle state of an insight based on its
    ///         age relative to lastConfirmedBlock.
    /// @param insightId The ID of the insight.
    /// @return The computed State.
    ///
    /// CRITICAL: Age is computed from lastConfirmedBlock, NOT publishBlock.
    /// This ensures that re-confirmation resets the decay clock. A prior audit
    /// found a bug where publishBlock was used, which meant re-confirming a
    /// DECAYING insight did not prevent further decay.
    ///
    /// Decay thresholds:
    ///   age > 1x  effective half-life  ->  DECAYING
    ///   age > 5x  effective half-life  ->  ARCHIVED
    ///   age > 10x effective half-life  ->  PURGED
    ///
    /// Special case for CHALLENGED:
    ///   age > 2x effective half-life   ->  ARCHIVED
    ///   (silence = sustained challenge; shorter than the normal 5x threshold)
    ///
    /// effective half-life = _kindHalfLife(kind) * _tierMultiplier(tier)
    ///
    /// This is a view function -- it does not modify storage. Callers who need
    /// the canonical on-chain state (e.g., for search result filtering) should
    /// call this function rather than reading insight.anchor.state directly.
    function computeState(uint256 insightId) public view returns (State) {
        Insight storage insight = insights[insightId];
        require(insight.anchor.publishBlock != 0, "Insight does not exist");

        uint64 age = uint64(block.number) - insight.lastConfirmedBlock;
        uint64 effectiveHalfLife = _kindHalfLife(Kind(insight.anchor.kind))
            * _tierMultiplier(Tier(insight.anchor.tier));

        // Decay thresholds (checked in order of severity, most severe first).
        if (age > effectiveHalfLife * 10) {
            return State.PURGED;
        }

        // CHALLENGED entries use a tighter timeline: 2x half-life -> ARCHIVED.
        // Rationale: if nobody defends a challenged insight for 2x its
        // half-life, the challenge is considered sustained.
        if (insight.anchor.state == uint8(State.CHALLENGED) &&
            age > effectiveHalfLife * 2) {
            return State.ARCHIVED;
        }

        if (age > effectiveHalfLife * 5) {
            return State.ARCHIVED;
        }

        if (age > effectiveHalfLife) {
            return State.DECAYING;
        }

        // Within active lifetime: return the stored state unchanged.
        // SUBMITTED stays SUBMITTED until confirmed externally.
        // ACTIVE stays ACTIVE. CHALLENGED stays CHALLENGED.
        return State(insight.anchor.state);
    }

    // -----------------------------------------------------------------------
    // renew()
    // -----------------------------------------------------------------------

    /// @notice Renew an ARCHIVED insight, transitioning it back to ACTIVE.
    ///         Resets publishBlock and lastConfirmedBlock. Requires fresh stake.
    /// @param insightId The ID of the insight to renew.
    function renew(uint256 insightId) external payable {
        Insight storage insight = insights[insightId];
        require(insight.anchor.publishBlock != 0, "Insight does not exist");
        require(
            insight.anchor.state == uint8(State.ARCHIVED),
            "Only ARCHIVED insights can be renewed"
        );
        require(msg.value >= MIN_STAKE, "Insufficient stake");

        insight.anchor.publishBlock = uint64(block.number);
        insight.lastConfirmedBlock = uint64(block.number);
        insight.stakedAmount += msg.value;

        _setState(insightId, insight, State.ACTIVE);
    }

    // -----------------------------------------------------------------------
    // purge()
    // -----------------------------------------------------------------------

    /// @notice Remove a PURGED insight from storage and the precompile index.
    ///         Returns 10% of the staked amount to the original author as a
    ///         "knowledge legacy" incentive.
    /// @param insightId The ID of the insight to purge.
    ///
    /// Anyone can call this. The caller benefits from SSTORE gas refunds
    /// (EIP-3529) when storage slots are zeroed.
    function purge(uint256 insightId) external {
        Insight storage insight = insights[insightId];
        require(insight.anchor.publishBlock != 0, "Insight does not exist");
        require(
            computeState(insightId) == State.PURGED,
            "Not yet purgeable"
        );

        address author = insight.anchor.author;
        uint256 legacy = insight.stakedAmount / 10; // 10% to original author

        // Remove vector from the HDC precompile index
        bytes32 vectorKey = keccak256(abi.encodePacked(insightId));
        HDC_PRECOMPILE.deleteVector(vectorKey);

        // Clear all storage (triggers SSTORE refund for caller)
        delete insights[insightId];

        // Transfer legacy to original author
        if (legacy > 0) {
            (bool ok, ) = author.call{value: legacy}("");
            require(ok, "Legacy transfer failed");
        }

        emit InsightStateChanged(insightId, State.PURGED, State.PURGED);
    }

    // -----------------------------------------------------------------------
    // View helpers
    // -----------------------------------------------------------------------

    /// @notice Delegate similarity search to the HDC precompile.
    function searchSimilar(bytes calldata queryVector, uint8 topK)
        external view returns (bytes32[] memory, uint16[] memory)
    {
        return HDC_PRECOMPILE.searchSimilar(queryVector, topK);
    }

    /// @notice Read the current state, accounting for time-based decay.
    function currentState(uint256 insightId) external view returns (State) {
        return computeState(insightId);
    }

    // -----------------------------------------------------------------------
    // Internal helpers
    // -----------------------------------------------------------------------

    /// @dev Emit InsightStateChanged and update stored state.
    function _setState(
        uint256 insightId,
        Insight storage insight,
        State newState
    ) internal {
        State oldState = State(insight.anchor.state);
        if (oldState != newState) {
            insight.anchor.state = uint8(newState);
            emit InsightStateChanged(insightId, oldState, newState);
        }
    }

    /// @dev Kind-specific base half-lives in blocks.
    ///      Block time is ~0.4s. Conversion: hours * 3600 / 0.4 = hours * 9000.
    ///
    ///      | Kind            | Hours | Blocks      |
    ///      |-----------------|-------|-------------|
    ///      | INSIGHT         | 72    |   648,000   |
    ///      | HEURISTIC       | 168   | 1,512,000   |
    ///      | ANTI_KNOWLEDGE  | 336   | 3,024,000   |
    ///      | WARNING         | 48    |   432,000   |
    ///      | CAUSAL_LINK     | 240   | 2,160,000   |
    ///      | STRATEGY        | 120   | 1,080,000   |
    function _kindHalfLife(Kind kind) internal pure returns (uint64) {
        if (kind == Kind.INSIGHT)        return   648_000;
        if (kind == Kind.HEURISTIC)      return 1_512_000;
        if (kind == Kind.ANTI_KNOWLEDGE) return 3_024_000;
        if (kind == Kind.WARNING)        return   432_000;
        if (kind == Kind.CAUSAL_LINK)    return 2_160_000;
        if (kind == Kind.STRATEGY)       return 1_080_000;
        revert("Unknown kind");
    }

    /// @dev Tier multipliers for effective half-life.
    ///
    ///      | Tier          | Multiplier |
    ///      |---------------|------------|
    ///      | TRANSIENT     | 1          |
    ///      | WORKING       | 3          |
    ///      | CONSOLIDATED  | 7          |
    ///      | PERSISTENT    | 10         |
    function _tierMultiplier(Tier tier) internal pure returns (uint64) {
        if (tier == Tier.TRANSIENT)    return 1;
        if (tier == Tier.WORKING)      return 3;
        if (tier == Tier.CONSOLIDATED) return 7;
        if (tier == Tier.PERSISTENT)   return 10;
        revert("Unknown tier");
    }
}
```

---

## Gas Cost Breakdown

### submit() -- ~88,400 gas

| Component | Gas | Notes |
|-----------|-----|-------|
| Base tx cost | 21,000 | Flat cost for any transaction |
| Calldata (bytes32 + bytes32 + uint8) | ~2,200 | 65 bytes; ~16 gas per non-zero byte |
| SSTORE slot 1 (vectorHash) | 22,100 | Cold, zero-to-nonzero |
| SSTORE slot 2 (contentHash) | 22,100 | Cold, zero-to-nonzero |
| SSTORE slot 3 (packed: author+publishBlock+kind+tier+state) | 22,100 | Cold, zero-to-nonzero |
| SSTORE slot 4 (confirmations+lastConfirmedBlock+confirmsSinceChallenge) | 0 | All zeros, no write needed |
| SSTORE slot 5 (stakedAmount) | ~2,900 | Non-zero value |
| SSTORE (_nextId increment) | ~2,900 | Warm, nonzero-to-nonzero |
| Event (InsightPublished, LOG2) | ~1,100 | 2 indexed topics + small data |
| **Total** | **~88,400** | Without precompile interaction |

If you also pass the full vector to the precompile for duplicate checking and
index storage, add approximately 50,000 gas for the precompile call.

### confirm() -- ~28,000 gas (base) / ~33,800 (with state + tier change)

| Component | Gas | Notes |
|-----------|-----|-------|
| Cold SLOAD (insight) | 2,100 | First read of the insight struct |
| Cold SSTORE (confirmers mapping) | 22,100 | New entry: zero-to-nonzero |
| Warm SSTORE (confirmations) | 2,900 | Nonzero-to-nonzero |
| Warm SSTORE (lastConfirmedBlock) | 2,900 | Nonzero-to-nonzero |
| Warm SSTORE (state change) | 2,900 | Only if state transitions |
| Warm SSTORE (tier promotion) | 2,900 | Only if tier threshold crossed |
| Event (InsightConfirmed, LOG2) | ~1,100 | |
| Event (InsightStateChanged, LOG2) | ~1,100 | Only if state changes |
| **Total (no transition)** | **~28,000** | |
| **Total (with state + tier)** | **~33,800** | |

### challenge() -- ~8,000 gas

| Component | Gas | Notes |
|-----------|-----|-------|
| Cold SLOAD (insight) | 2,100 | |
| Warm SSTORE (state) | 2,900 | ACTIVE -> CHALLENGED |
| Warm SSTORE (confirmsSinceChallenge) | 2,900 | Reset to 0 |
| Event (InsightChallenged, LOG2) | ~1,100 | |
| Event (InsightStateChanged, LOG2) | ~1,100 | |
| **Total** | **~8,000** | |

### computeState() -- 0 gas (view function)

This is a `view` function. When called via `eth_call`, there is no gas cost
to the caller. The node computes it locally.

### purge() -- ~15,000 gas (with SSTORE refunds)

The caller pays for the precompile `deleteVector` call and the `delete`
operation, but receives SSTORE refunds (EIP-3529) for zeroing 5 storage slots.
Net cost is approximately 15,000 gas after refunds.

---

## Security Considerations

### 1. Reentrancy

The `purge()` function sends ETH to the original author via a low-level
`.call{value: legacy}("")`. This is a reentrancy risk. The code uses a
checks-effects-interactions pattern:

1. **Checks:** Verify the insight exists and is purgeable.
2. **Effects:** Delete storage (`delete insights[insightId]`) BEFORE
   the external call.
3. **Interactions:** Transfer ETH last.

Because the insight is deleted before the call, a reentrant call to `purge()`
for the same ID will fail the existence check. This is sufficient for
single-function reentrancy. For defense-in-depth, consider adding a
`nonReentrant` modifier (OpenZeppelin's ReentrancyGuard).

### 2. Access Control

The current design has **no access control** on `challenge()`. Any address can
challenge any ACTIVE insight. In the full system, challenges should ideally
require the challenger to have published anti-knowledge (Kind.ANTI_KNOWLEDGE)
whose vector resonates with the target. Without this gate, griefing attacks are
cheap (~8,000 gas to challenge).

Options:
- Require `msg.sender` to have published ANTI_KNOWLEDGE with vector similarity
  > 0.7 to the target (validated via precompile).
- Require a small challenge stake that is burned if the challenge is resolved
  (5+ confirmations restore ACTIVE).

### 3. Front-Running

An attacker watching the mempool could see a `submit()` transaction, extract
the vector hash and content hash, and front-run with their own submission to
claim authorship. Mitigations:
- Use commit-reveal: first commit a hash of (vectorHash, contentHash, salt),
  then reveal in a subsequent transaction.
- Use a private mempool (e.g., Flashbots Protect equivalent on daeji).
- Accept the risk if the chain is private/consortium with trusted validators.

### 4. Integer Overflow

Solidity 0.8+ has built-in overflow checking. All arithmetic operations will
revert on overflow. The `uint64` types for block numbers and confirmation counts
are sufficient:
- `uint64` max = 18,446,744,073,709,551,615
- At 0.4s per block, `uint64` overflows after ~234 billion years
- Confirmations reaching `uint64` max is physically impossible

### 5. Denial of Service via Dust Insights

An attacker could spam `submit()` with MIN_STAKE to fill storage. Mitigations:
- MIN_STAKE acts as an economic barrier (0.01 ETH per insight).
- Insights naturally decay and can be purged, freeing storage.
- Consider rate-limiting submissions per address per block.

---

## Anti-Patterns -- What NOT to Do

These are specific bugs found in prior audits. Do not repeat them.

### Anti-Pattern 1: Accepting Only ACTIVE in confirm()

```solidity
// BAD -- breaks DECAYING->ACTIVE recovery and CHALLENGED->ACTIVE resolution
function confirm(uint256 insightId) external {
    require(insight.anchor.state == uint8(State.ACTIVE), "Not active");
    // ...
}
```

The `confirm()` function MUST accept SUBMITTED, ACTIVE, DECAYING, and
CHALLENGED states. The DECAYING->ACTIVE path is the primary mechanism for
reviving neglected knowledge. The CHALLENGED->ACTIVE path (5+ confirmations)
is the community dispute resolution mechanism.

### Anti-Pattern 2: Using publishBlock for Age in computeState()

```solidity
// BAD -- re-confirmation does not reset the decay clock
function computeState(uint256 insightId) public view returns (State) {
    uint64 age = uint64(block.number) - insight.anchor.publishBlock; // WRONG
    // ...
}
```

Age MUST be computed from `lastConfirmedBlock`, not `publishBlock`. If you use
`publishBlock`, then confirming a DECAYING insight transitions it to ACTIVE (via
`confirm()`), but `computeState()` still sees a large age and immediately
returns DECAYING again. The re-confirmation has no effect on decay, which
defeats the purpose of the recovery path.

### Anti-Pattern 3: Forgetting the Existence Check

```solidity
// BAD -- allows phantom writes to non-existent insight IDs
function confirm(uint256 insightId) external {
    Insight storage insight = insights[insightId];
    // No existence check here!
    // A non-existent insight has default values: publishBlock=0, state=0 (SUBMITTED)
    // The state check below passes because SUBMITTED is a valid confirmable state.
    require(insight.anchor.state == uint8(State.SUBMITTED) || ...);
    insight.confirmations++;  // Writes to storage for a phantom insight
}
```

Always check `insight.anchor.publishBlock != 0` before any operation. A
non-existent insight has all-zero fields, and `State.SUBMITTED` (value 0)
is a valid confirmable state, so the state check alone is insufficient.

### Anti-Pattern 4: Using Floating Point

```solidity
// BAD -- consensus violation: f64 results vary by platform
function computeDecay(uint256 insightId) public view returns (uint256) {
    uint256 age = block.number - insight.lastConfirmedBlock;
    return uint256(0.5 ** (age / halfLife));  // Solidity does not support this
}
```

Solidity has no floating-point type. All decay computation uses integer
arithmetic with block-number thresholds. The `computeState()` function uses
simple comparisons (`age > effectiveHalfLife * N`) rather than exponential
decay formulas.

---

## Test Plan (Foundry)

### Setup

```solidity
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.20;

import "forge-std/Test.sol";
import "../src/InsightBoard.sol";

contract InsightBoardTest is Test {
    InsightBoard board;
    address alice = makeAddr("alice");
    address bob = makeAddr("bob");
    address carol = makeAddr("carol");

    function setUp() public {
        board = new InsightBoard();
        vm.deal(alice, 10 ether);
        vm.deal(bob, 10 ether);
        vm.deal(carol, 10 ether);
    }

    // Helper: submit an insight as the given user
    function _submit(address user, uint8 kind) internal returns (uint256) {
        vm.prank(user);
        return board.submit{value: 0.01 ether}(
            keccak256("vector"),
            keccak256("content"),
            kind
        );
    }
}
```

### Test Cases

#### 1. Submit -- Happy Path

```solidity
function test_submit_creates_insight() public {
    uint256 id = _submit(alice, 0); // Kind.INSIGHT
    (
        InsightBoard.InsightAnchor memory anchor,
        uint64 confirmations,
        uint64 lastConfirmedBlock,
        uint64 confirmsSinceChallenge,
        uint256 stakedAmount
    ) = board.insights(id);

    assertEq(anchor.author, alice);
    assertEq(anchor.state, uint8(InsightBoard.State.SUBMITTED));
    assertEq(anchor.tier, uint8(InsightBoard.Tier.TRANSIENT));
    assertEq(anchor.kind, 0);
    assertEq(confirmations, 0);
    assertEq(stakedAmount, 0.01 ether);
}
```

#### 2. Submit -- Revert on Insufficient Stake

```solidity
function test_submit_reverts_insufficient_stake() public {
    vm.prank(alice);
    vm.expectRevert("Insufficient stake");
    board.submit{value: 0.001 ether}(
        keccak256("v"), keccak256("c"), 0
    );
}
```

#### 3. Submit -- Revert on Invalid Kind

```solidity
function test_submit_reverts_invalid_kind() public {
    vm.prank(alice);
    vm.expectRevert("Invalid kind");
    board.submit{value: 0.01 ether}(
        keccak256("v"), keccak256("c"), 6 // Out of range
    );
}
```

#### 4. Confirm -- SUBMITTED to ACTIVE

```solidity
function test_confirm_submitted_to_active() public {
    uint256 id = _submit(alice, 0);

    vm.prank(bob);
    board.confirm(id);

    (, , , , ) = board.insights(id);
    assertEq(
        uint8(board.computeState(id)),
        uint8(InsightBoard.State.ACTIVE)
    );
}
```

#### 5. Confirm -- Double Confirmation Reverts

```solidity
function test_confirm_double_reverts() public {
    uint256 id = _submit(alice, 0);

    vm.prank(bob);
    board.confirm(id);

    vm.prank(bob);
    vm.expectRevert("Already confirmed");
    board.confirm(id);
}
```

#### 6. Confirm -- Non-Existent Insight Reverts

```solidity
function test_confirm_nonexistent_reverts() public {
    vm.prank(bob);
    vm.expectRevert("Insight does not exist");
    board.confirm(999);
}
```

#### 7. Confirm -- DECAYING to ACTIVE Recovery

```solidity
function test_confirm_decaying_to_active() public {
    uint256 id = _submit(alice, 0); // Kind.INSIGHT, half-life = 648,000 blocks

    vm.prank(bob);
    board.confirm(id); // SUBMITTED -> ACTIVE

    // Fast-forward past 1x half-life
    vm.roll(block.number + 648_001);
    assertEq(
        uint8(board.computeState(id)),
        uint8(InsightBoard.State.DECAYING)
    );

    // Re-confirm to recover
    vm.prank(carol);
    board.confirm(id); // DECAYING -> ACTIVE

    assertEq(
        uint8(board.computeState(id)),
        uint8(InsightBoard.State.ACTIVE)
    );
}
```

#### 8. Confirm -- CHALLENGED to ACTIVE Resolution (5+ confs)

```solidity
function test_confirm_challenged_to_active() public {
    uint256 id = _submit(alice, 0);

    vm.prank(bob);
    board.confirm(id); // SUBMITTED -> ACTIVE

    vm.prank(carol);
    board.challenge(id); // ACTIVE -> CHALLENGED

    // Send 5 confirmations from different addresses
    for (uint256 i = 0; i < 5; i++) {
        address confirmer = makeAddr(string(abi.encodePacked("confirmer", i)));
        vm.deal(confirmer, 1 ether);
        vm.prank(confirmer);
        board.confirm(id);
    }

    assertEq(
        uint8(board.computeState(id)),
        uint8(InsightBoard.State.ACTIVE)
    );
}
```

#### 9. Challenge -- Happy Path

```solidity
function test_challenge_active_to_challenged() public {
    uint256 id = _submit(alice, 0);

    vm.prank(bob);
    board.confirm(id); // SUBMITTED -> ACTIVE

    vm.prank(carol);
    board.challenge(id); // ACTIVE -> CHALLENGED

    (InsightBoard.InsightAnchor memory anchor, , , , ) = board.insights(id);
    assertEq(anchor.state, uint8(InsightBoard.State.CHALLENGED));
}
```

#### 10. Challenge -- Revert on Non-ACTIVE

```solidity
function test_challenge_submitted_reverts() public {
    uint256 id = _submit(alice, 0);

    vm.prank(bob);
    vm.expectRevert("Only ACTIVE insights can be challenged");
    board.challenge(id); // Still SUBMITTED
}
```

#### 11. computeState -- CHALLENGED to ARCHIVED at 2x Half-Life

```solidity
function test_computeState_challenged_to_archived() public {
    uint256 id = _submit(alice, 0); // Kind.INSIGHT

    vm.prank(bob);
    board.confirm(id); // SUBMITTED -> ACTIVE

    vm.prank(carol);
    board.challenge(id); // ACTIVE -> CHALLENGED

    // Fast-forward past 2x half-life (2 * 648,000 = 1,296,000 blocks)
    vm.roll(block.number + 1_296_001);

    assertEq(
        uint8(board.computeState(id)),
        uint8(InsightBoard.State.ARCHIVED)
    );
}
```

#### 12. computeState -- Decay Uses lastConfirmedBlock, Not publishBlock

```solidity
function test_computeState_uses_lastConfirmedBlock() public {
    uint256 id = _submit(alice, 0);

    vm.prank(bob);
    board.confirm(id); // SUBMITTED -> ACTIVE, sets lastConfirmedBlock

    // Fast-forward to just before 1x half-life
    vm.roll(block.number + 647_999);
    assertEq(
        uint8(board.computeState(id)),
        uint8(InsightBoard.State.ACTIVE)
    );

    // Re-confirm, resetting lastConfirmedBlock
    vm.prank(carol);
    board.confirm(id);

    // Fast-forward another 647,999 blocks from the new confirmation
    vm.roll(block.number + 647_999);

    // Should STILL be ACTIVE because lastConfirmedBlock was reset
    assertEq(
        uint8(board.computeState(id)),
        uint8(InsightBoard.State.ACTIVE)
    );
}
```

#### 13. Tier Promotion at Thresholds

```solidity
function test_tier_promotion_at_3_10_25() public {
    uint256 id = _submit(alice, 0);

    // 3 confirmations -> WORKING
    for (uint256 i = 0; i < 3; i++) {
        address c = makeAddr(string(abi.encodePacked("t", i)));
        vm.deal(c, 1 ether);
        vm.prank(c);
        board.confirm(id);
    }
    (InsightBoard.InsightAnchor memory a1, , , , ) = board.insights(id);
    assertEq(a1.tier, uint8(InsightBoard.Tier.WORKING));

    // 10 confirmations total -> CONSOLIDATED
    for (uint256 i = 3; i < 10; i++) {
        address c = makeAddr(string(abi.encodePacked("t", i)));
        vm.deal(c, 1 ether);
        vm.prank(c);
        board.confirm(id);
    }
    (InsightBoard.InsightAnchor memory a2, , , , ) = board.insights(id);
    assertEq(a2.tier, uint8(InsightBoard.Tier.CONSOLIDATED));

    // 25 confirmations total -> PERSISTENT
    for (uint256 i = 10; i < 25; i++) {
        address c = makeAddr(string(abi.encodePacked("t", i)));
        vm.deal(c, 1 ether);
        vm.prank(c);
        board.confirm(id);
    }
    (InsightBoard.InsightAnchor memory a3, , , , ) = board.insights(id);
    assertEq(a3.tier, uint8(InsightBoard.Tier.PERSISTENT));
}
```

#### 14. Purge -- Happy Path

```solidity
function test_purge_after_10x_halflife() public {
    uint256 id = _submit(alice, 3); // Kind.WARNING, half-life = 432,000 blocks

    vm.prank(bob);
    board.confirm(id); // SUBMITTED -> ACTIVE

    // Fast-forward past 10x half-life (4,320,000 blocks)
    vm.roll(block.number + 4_320_001);

    assertEq(
        uint8(board.computeState(id)),
        uint8(InsightBoard.State.PURGED)
    );

    uint256 authorBalBefore = alice.balance;
    vm.prank(bob);
    board.purge(id);

    // Author should receive 10% legacy (0.001 ether)
    assertEq(alice.balance, authorBalBefore + 0.001 ether);
}
```

#### 15. Purge -- Revert if Not Purgeable

```solidity
function test_purge_reverts_if_not_purgeable() public {
    uint256 id = _submit(alice, 0);

    vm.prank(bob);
    board.confirm(id);

    vm.prank(carol);
    vm.expectRevert("Not yet purgeable");
    board.purge(id);
}
```

#### 16. Renew -- ARCHIVED to ACTIVE

```solidity
function test_renew_archived_to_active() public {
    uint256 id = _submit(alice, 0);

    vm.prank(bob);
    board.confirm(id);

    // Fast-forward past 5x half-life to reach ARCHIVED
    vm.roll(block.number + 648_000 * 5 + 1);
    assertEq(
        uint8(board.computeState(id)),
        uint8(InsightBoard.State.ARCHIVED)
    );

    // Manually set stored state to ARCHIVED (computeState is view-only)
    // In production, a state-syncing function or keeper would do this.
    // For testing, we directly set the state:
    // NOTE: In the test, we need the stored state to be ARCHIVED.
    // computeState() returns ARCHIVED but the stored state may still be ACTIVE.
    // The renew() function checks insight.anchor.state, not computeState().
    // This is a design consideration -- see note below.
}
```

> **Design note on renew():** The `renew()` function checks the stored
> `insight.anchor.state`, not `computeState()`. You may need a
> `syncState(uint256 insightId)` function that writes the computed state back
> to storage, or modify `renew()` to check `computeState()` instead. The
> reference implementation in doc 07 checks `insight.anchor.state`, so
> consider adding a `materializeState()` function that anyone can call to
> update stored state to match `computeState()`.

---

## Implementation Checklist

### Core Contract

- [x] Define `Kind`, `State`, and `Tier` enums with exact values matching the spec
- [x] Define `InsightAnchor` struct (verify 3-slot packing: 32 + 32 + 31 bytes)
- [x] Define `Insight` struct with `confirmsSinceChallenge` field
- [x] Implement `submit()` with MIN_STAKE check, kind validation, and event emission (F03: signature changed to raw bytes)
- [x] Implement `confirm()` accepting SUBMITTED, ACTIVE, DECAYING, and CHALLENGED
- [x] Implement tier promotion in `confirm()` at 3/10/25 thresholds
- [x] Implement CHALLENGED->ACTIVE resolution at 5+ `confirmsSinceChallenge` (extracted to `CHALLENGE_RESOLUTION_CONFS` constant)
- [x] Implement `challenge()` restricted to ACTIVE state (F06: enhanced with anti-knowledge reference requirement)
- [x] Implement `computeState()` using `lastConfirmedBlock` (NOT `publishBlock`)
- [x] Implement CHALLENGED->ARCHIVED at 2x half-life in `computeState()`
- [x] Implement `renew()` for ARCHIVED->ACTIVE with fresh stake (**BROKEN: checks stored state, not computed state -- F09**)
- [x] Implement `purge()` with 10% legacy return and storage deletion (F11: CEI violation with precompile call before delete)
- [x] Add existence check (`publishBlock != 0`) to every function (F08: `computeState()` returns PURGED instead of reverting)
- [x] Add `_kindHalfLife()` with all 6 kind values -- exact match
- [x] Add `_tierMultiplier()` with all 4 tier values -- exact match

### Events

- [x] `InsightPublished` emitted in `submit()` (F05: richer signature with vector+content bytes)
- [x] `InsightConfirmed` emitted in `confirm()` (F05: uint64 vs spec's uint32)
- [x] `InsightChallenged` emitted in `challenge()` (F05: includes `challengingInsightId`)
- [x] `InsightStateChanged` emitted on every state transition (F12: **missing from `purge()`**)
- [x] `InsightRenewed` emitted -- new event not in spec (positive addition)
- [x] `InsightPurged` emitted -- new event not in spec (positive addition)

### Security

- [x] Checks-effects-interactions pattern in `purge()` -- **partially broken** (F11: precompile call before delete)
- [x] No floating-point arithmetic anywhere
- [x] `confirmers` mapping prevents double-confirmation
- [ ] Add `ReentrancyGuard` to `purge()` -- **NOT DONE** (SEC01)
- [x] Access control on `challenge()` -- requires anti-knowledge reference (F06), but **missing resonance check** (F07)

### Tests

- [x] Submit happy path
- [x] Submit reverts on insufficient stake
- [x] Submit reverts on invalid kind
- [x] Confirm SUBMITTED -> ACTIVE
- [x] Confirm double-confirmation reverts
- [x] Confirm non-existent insight reverts
- [x] Confirm DECAYING -> ACTIVE recovery
- [x] Confirm CHALLENGED -> ACTIVE at 5+ confs
- [x] Challenge ACTIVE -> CHALLENGED
- [x] Challenge non-ACTIVE reverts
- [x] computeState CHALLENGED -> ARCHIVED at 2x half-life
- [x] computeState uses lastConfirmedBlock, not publishBlock
- [x] Tier promotion at 3, 10, 25 confirmations
- [x] Purge after 10x half-life
- [x] Purge reverts if not purgeable
- [x] Purge sends 10% legacy to author
- [ ] Renew ARCHIVED -> ACTIVE -- **test incomplete** (cannot trigger stored ARCHIVED state without `materializeState()`)

### Integration

- [x] Wire `submit()` to HDC precompile `storeVector()` -- done via `HdcLib` library
- [x] Wire `submit()` duplicate check via precompile `searchSimilar()` -- done (AP02: **skipped in test mock**)
- [x] Wire `purge()` to HDC precompile `deleteVector()` -- done via `HdcLib`
- [x] Verify `searchSimilar()` delegation works -- delegated to `HdcLib`

---

## Prioritized Fix Checklist

Ordered by severity. Items reference audit findings (F01-F16) and anti-patterns (AP01-AP05).

### CRITICAL

- [ ] **Fix stored vs computed state bug (F09, F10, AP04):** `renew()` checks `insight.anchor.state == ARCHIVED` but `computeState()` is the authoritative state function. An insight whose stored state is ACTIVE but whose computed state is ARCHIVED (due to time decay) cannot be renewed. This makes `renew()` unreachable for insights that naturally decay. **Fix:** Implement `materializeState(uint256 insightId)` that writes `computeState(insightId)` back to `insight.anchor.state`. This is a public function anyone can call to sync stored state with computed state. Alternatively, modify `renew()` and `confirm()` to call `computeState()` internally before checking state.

```solidity
/// @notice Sync stored state with computed state (time-based transitions).
/// @dev Anyone can call this to materialize time-based decay into storage.
function materializeState(uint256 insightId) external {
    Insight storage insight = insights[insightId];
    require(insight.anchor.publishBlock != 0, "Insight does not exist");
    State computed = computeState(insightId);
    _setState(insightId, insight, computed);
}
```

- [ ] **Add `materializeState()` or change `renew()`/`confirm()` to use `computeState()`:** Until this is done, the stored-vs-computed inconsistency means event listeners get inaccurate state data for time-based transitions, and `renew()` is a dead code path for naturally-decayed insights.

### HIGH

- [ ] **Implement resonance check in `challenge()` with `RESONANCE_THRESHOLD` (F07):** The constant `RESONANCE_THRESHOLD = 1024` is declared but never used. Add a Hamming distance check between the target insight's vector and the anti-knowledge's vector. Challenges with unrelated anti-knowledge should be rejected.

```solidity
// In challenge():
uint16 distance = HdcLib.hamming(
    insights[targetInsightId].anchor.vectorHash,
    insights[antiKnowledgeId].anchor.vectorHash
);
require(distance <= RESONANCE_THRESHOLD, "Anti-knowledge not resonant with target");
```

- [ ] **Fix test mocks (AP01, AP02):** `TestableInsightBoard` duplicates `submit()` and `purge()` function bodies with precompile calls removed. Any bug fix to the real contract must be manually duplicated in the test mock. **Fix:** Use `vm.mockCall()` or `vm.etch()` to mock the precompile at address `0x09`, then test the real `InsightBoard` contract directly:

```solidity
// Mock the precompile to return success for storeVector
vm.mockCall(
    address(0x09),
    abi.encodePacked(uint8(0x05)),  // storeVector selector
    abi.encode(true)
);
// Mock searchSimilar to return no duplicates
vm.mockCall(
    address(0x09),
    abi.encodePacked(uint8(0x06)),  // searchSimilar selector
    abi.encode(new bytes32[](0), new uint16[](0))
);
```

### MEDIUM

- [ ] **Add `ReentrancyGuard` to `purge()` (SEC01):** The `purge()` function has an external call to the precompile before storage deletion, and an ETH transfer to the author. Add OpenZeppelin's `ReentrancyGuard` as defense-in-depth. Also reorder to put `delete insights[insightId]` before `HdcLib.deleteVector()` for stricter CEI.

- [ ] **Emit `InsightStateChanged` in `purge()` (F12):** The spec expects `InsightStateChanged(insightId, State.PURGED, State.PURGED)`. The implementation only emits `InsightPurged`. Add the `InsightStateChanged` emission for listeners that generically track all state transitions.

- [ ] **Address locked ETH (SEC05):** 90% of purged stakes are permanently locked in the contract. If intentional (deflationary), add documentation. If not, implement a treasury withdrawal or redistribution mechanism.

### LOW

- [ ] **Add `currentState()` wrapper:** The spec defines `currentState(insightId)` as a convenience view function wrapping `computeState()`. Implementation omits it. Add for API completeness.

- [ ] **Remove dead constant `RESONANCE_THRESHOLD`** if resonance check is not being implemented, OR implement the check (HIGH item above).

- [ ] **Fix `InsightView` test fragility (AP05):** The test-only `InsightView` struct must be manually synced with `Insight`. Consider adding a proper getter to the contract or interface.

---

## Verification

### Source files

| File | Role |
|------|------|
| `contracts/src/InsightBoard.sol` | Main contract implementation |
| `contracts/src/IInsightBoard.sol` | Interface with enums, events, function signatures |
| `contracts/src/HdcPrecompile.sol` | `HdcLib` library wrapping precompile calls |
| `contracts/test/InsightBoard.t.sol` | Foundry test suite |
| `contracts/foundry.toml` | Build config (solc 0.8.28, via_ir=true, 200 optimizer runs) |

### Running tests

```bash
# From contracts/ directory
cd contracts && forge test -vv

# Run specific test
forge test -vv --match-test test_confirm_decaying_to_active

# Gas report
forge test --gas-report

# Expected: 16+ test functions passing
```

### PR #42 relationship

PR #42's event decoder in `crates/hdc/chain/src/event.rs` parses
`InsightPublished` events to rebuild the vector index. The decoder depends on
the event signature from `IInsightBoard.sol`. If event signatures change (e.g.,
fixing F05), the decoder must be updated in sync.

---

## Audit Findings

Audit performed 2026-05-08 against the implementation files:
- `/Users/will/dev/nunchi/daeji/contracts/src/InsightBoard.sol`
- `/Users/will/dev/nunchi/daeji/contracts/src/IInsightBoard.sol`
- `/Users/will/dev/nunchi/daeji/contracts/src/HdcPrecompile.sol`
- `/Users/will/dev/nunchi/daeji/contracts/test/InsightBoard.t.sol`
- `/Users/will/dev/nunchi/daeji/contracts/foundry.toml`

### F01 -- Architecture Divergence: Interface + Library vs Monolithic Contract

The spec defines a monolithic `InsightBoard` contract with an `IHdcPrecompile`
interface for the precompile. The implementation splits this into three files:

1. **`IInsightBoard.sol`** -- a full interface with enums, events, and function
   signatures. The spec has no interface contract; enums and events live inside
   the contract itself.
2. **`HdcPrecompile.sol`** -- a `HdcLib` library that wraps precompile calls
   using raw `abi.encodePacked` selector dispatch (selectors `0x01`..`0x07`).
   The spec uses a typed Solidity interface `IHdcPrecompile`.
3. **`InsightBoard.sol`** -- the implementation, which `is IInsightBoard` and
   imports `HdcLib`.

**Verdict:** This is a reasonable architectural improvement over the spec. The
interface enables external tooling and type-safe integration. The library
approach for the precompile is justified because precompiles do not expose
standard Solidity ABI -- raw calldata encoding is the correct pattern.

### F02 -- ID Scheme Change: bytes32 Hash vs uint256 Auto-Increment

| Aspect | Spec | Implementation |
|--------|------|----------------|
| ID type | `uint256` (auto-increment via `_nextId`) | `bytes32` (hash of `vectorHash + msg.sender + block.number`) |
| Collision handling | None needed (monotonic) | `require(insights[insightId].anchor.publishBlock == 0)` |
| Storage | `_nextId` state var costs one slot | No counter needed |

**Files:**
- Spec: lines 278-279 (`uint256 private _nextId;`)
- Impl: `InsightBoard.sol` lines 89-91

**Verdict:** The hash-based ID is a design improvement. It removes sequential
enumeration (harder to scrape) and eliminates the `_nextId` storage slot. The
collision guard on line 94-97 is correct. However, this creates a subtle risk:
if the same author submits the same vector in the same block, the ID collides.
This is unlikely but not impossible in high-throughput scenarios.

### F03 -- submit() Signature Change: Raw Bytes vs Pre-Hashed

| Aspect | Spec | Implementation |
|--------|------|----------------|
| Parameters | `bytes32 vectorHash, bytes32 contentHash, uint8 kind` | `Kind kind, bytes calldata vector, bytes calldata content` |
| Hashing | Caller pre-hashes | Contract hashes via `keccak256(vector)` and `keccak256(content)` |
| Calldata cost | ~2,200 gas (65 bytes) | ~20,800+ gas (1,280 + content bytes) |
| Full data in event | Optional (spec note) | Always included (`vector` and `content` in `InsightPublished`) |

**Files:**
- Spec: line 336 (`function submit(bytes32, bytes32, uint8)`)
- Impl: `InsightBoard.sol` line 68-72

**Verdict:** This is a deliberate design choice. Passing raw bytes enables
on-chain vector validation (length check on line 73), precompile interaction
(duplicate search on line 79-86, store on line 117), and full event-log
reconstruction. The tradeoff is significantly higher gas cost per submission.
The spec notes this as optional ("add ~13,350 gas ... for event-log
reconstruction"). The implementation makes it mandatory.

### F04 -- submit() Emits Two Events Instead of One

The spec emits only `InsightPublished`. The implementation emits both
`InsightPublished` (line 120) and `InsightStateChanged` (line 130) with
`oldState = type(uint8).max` (sentinel for "new entry"). This is consistent
with the interface definition and is a positive addition -- it ensures state
change listeners receive the initial transition without special-casing.

### F05 -- Event Signature Differences

| Event | Spec | Implementation |
|-------|------|----------------|
| `InsightPublished` | `(uint256 indexed id, address indexed author, bytes32 vectorHash, uint8 kind)` | `(bytes32 indexed insightId, bytes32 indexed vectorHash, address indexed author, bytes vector, bytes content, uint8 kind, uint8 tier)` |
| `InsightConfirmed` | `(uint256 indexed id, address indexed confirmer, uint32 confirmations)` | `(bytes32 indexed insightId, address indexed confirmer, uint64 totalConfirmations)` |
| `InsightChallenged` | `(uint256 indexed id, address indexed challenger)` | `(bytes32 indexed insightId, bytes32 indexed challengingInsightId, address indexed challenger)` |
| `InsightStateChanged` | `(uint256 indexed id, State oldState, State newState)` | `(bytes32 indexed insightId, uint8 oldState, uint8 newState)` |
| `InsightRenewed` | Not in spec | `(bytes32 indexed insightId, address indexed renewer)` |
| `InsightPurged` | Not in spec (only `InsightStateChanged(PURGED, PURGED)`) | `(bytes32 indexed insightId, address indexed purger)` |

**Files:**
- Spec: lines 188-211
- Impl: `IInsightBoard.sol` lines 46-81

**Verdict:** The implementation events are richer and more useful. Key
improvements: `InsightPublished` includes `vectorHash` as indexed (enables
filtering), full `vector` and `content` bytes for event replay, `tier` for
initial tier. `InsightChallenged` includes the `challengingInsightId` which
enables linking the anti-knowledge entry. `InsightRenewed` and `InsightPurged`
are new dedicated events that are cleaner than overloading `InsightStateChanged`.

**Concern:** `InsightPublished` has 3 indexed params (max for non-anonymous
events is 3 plus the topic0 selector), which is correct. However, including the
full 1,280-byte `vector` in the event data is expensive (~20K+ gas for LOG data
alone). The spec acknowledged this cost and made it optional.

### F06 -- challenge() Requires Anti-Knowledge Reference

| Aspect | Spec | Implementation |
|--------|------|----------------|
| Signature | `challenge(uint256 insightId)` | `challenge(bytes32 targetInsightId, bytes32 antiKnowledgeId)` |
| Validation | None (any caller can challenge) | Requires `antiKnowledgeId` to exist AND have `kind == ANTI_KNOWLEDGE` |
| Access control | Open (spec flags this as a security concern) | Gated by anti-knowledge existence |

**Files:**
- Spec: line 481 (`function challenge(uint256 insightId) external`)
- Impl: `InsightBoard.sol` lines 197-234

**Verdict:** This directly addresses the spec's Security Consideration #2
("Access Control" on lines 772-783). The spec recommends requiring the
challenger to have published anti-knowledge. The implementation does this.
However, it does NOT verify HDC vector resonance between the target and
anti-knowledge (the spec suggests similarity > 0.7). Any `ANTI_KNOWLEDGE`
entry can challenge any `ACTIVE` entry, regardless of semantic relationship.

### F07 -- challenge() Missing Resonance Check

The spec suggests (line 779): "Require `msg.sender` to have published
ANTI_KNOWLEDGE with vector similarity > 0.7 to the target (validated via
precompile)." The contract defines `RESONANCE_THRESHOLD = 1024` (line 27) but
never uses it. The `challenge()` function does not call `HdcLib.hamming()` or
`HdcLib.searchSimilar()` to verify the anti-knowledge vector is semantically
related to the target.

**File:** `InsightBoard.sol` line 27 (constant) and lines 197-234 (function)

**Severity:** Medium. Without resonance validation, any published anti-knowledge
can challenge any active insight, enabling griefing with unrelated entries.

### F08 -- computeState() Behavior for Non-Existent Insights

| Aspect | Spec | Implementation |
|--------|------|----------------|
| Non-existent ID | `require(publishBlock != 0, "Insight does not exist")` (reverts) | Returns `State.PURGED` (lines 305-307) |

**Files:**
- Spec: line 525
- Impl: `InsightBoard.sol` lines 303-307

**Verdict:** The implementation silently returns `PURGED` for non-existent
insights. The spec reverts. The implementation's approach is arguably more
useful for callers (no need to catch reverts when batch-querying states), but
it is a semantic difference: callers cannot distinguish "purged and removed"
from "never existed." The `purge()` function does its own existence check
(line 267) so this does not create a security hole there.

### F09 -- renew() Checks Stored State, Not Computed State

The `renew()` function on line 244-245 checks:
```solidity
require(insight.anchor.state == uint8(State.ARCHIVED), ...)
```

But `computeState()` is the authoritative state function. An insight whose
stored state is `ACTIVE` but whose computed state is `ARCHIVED` (due to time
decay) cannot be renewed because the stored state is still `ACTIVE`. The spec
notes this exact problem on lines 1249-1255 and suggests a `materializeState()`
or `syncState()` function. The implementation does not provide one.

**File:** `InsightBoard.sol` lines 244-245

**Severity:** High. Without a `syncState()` / `materializeState()` function,
there is no on-chain path to transition an insight's stored state from ACTIVE
to ARCHIVED. This means `renew()` is unreachable for insights that naturally
decay without explicit state changes. The only way an insight reaches stored
`ARCHIVED` state is if some other mechanism writes it -- but no such mechanism
exists in the contract.

### F10 -- confirm() Checks Stored State, Not Computed State

Similarly, `confirm()` on lines 154-161 checks the stored
`insight.anchor.state`. An insight whose stored state is `ACTIVE` but whose
computed state is `DECAYING` will be confirmed as if it is `ACTIVE` (no state
transition to emit), even though `computeState()` would report `DECAYING`.

This is partially mitigated because confirming an `ACTIVE` insight still resets
`lastConfirmedBlock`, which effectively re-activates it. But the
`InsightStateChanged` event will NOT be emitted (because the stored state never
changed), so event listeners will miss the recovery.

**File:** `InsightBoard.sol` lines 154-161

**Severity:** Low-Medium. Functionally correct (the refresh works), but event
log is inaccurate for monitoring.

### F11 -- purge() Violates CEI for Event Emission

The spec's `purge()` (line 616) emits `InsightStateChanged` after `delete`.
The implementation's `purge()` (lines 263-293) follows this order:

1. Cache author and legacy amount (lines 276-277)
2. Call `HdcLib.deleteVector()` -- **external call** (line 280)
3. Delete storage (line 283)
4. Emit `InsightPurged` (line 286)
5. Transfer ETH (lines 289-291)

**Issue:** The `HdcLib.deleteVector()` call on line 280 is an external call
(`HDC_PRECOMPILE.call(...)`) that happens BEFORE `delete insights[insightId]`.
This means if the precompile were malicious or had a callback mechanism, it
could re-enter. The precompile is trusted infrastructure (address `0x09`), so
this is low risk in practice, but it violates the strict CEI pattern the code
comments claim on line 262 and line 99.

**File:** `InsightBoard.sol` lines 262-293

**Severity:** Low (precompile is trusted), but the code comment is misleading.

### F12 -- Purge Does Not Emit InsightStateChanged

The spec's `purge()` emits `InsightStateChanged(insightId, State.PURGED,
State.PURGED)` (spec line 616). The implementation emits only `InsightPurged`
(line 286) and does not emit `InsightStateChanged`. Listeners that rely on
`InsightStateChanged` for all transitions will miss the final purge event.

**File:** `InsightBoard.sol` line 286

**Severity:** Low. The dedicated `InsightPurged` event serves the same purpose,
but listeners that generically track `InsightStateChanged` will have an
incomplete picture.

### F13 -- CHALLENGE_RESOLUTION_CONFS Constant

The spec hardcodes `5` in the `confirm()` function (line 435:
`if (insight.confirmsSinceChallenge >= 5)`). The implementation extracts this
to a named constant `CHALLENGE_RESOLUTION_CONFS = 5` (line 24) and uses it
on line 178. This is a minor improvement for readability and configurability.

### F14 -- HdcLib Exposes Extra Operations Not in Spec

The `HdcLib` library (`HdcPrecompile.sol`) exposes 6 functions:
`hamming`, `bind`, `bundle`, `permute`, `storeVector`, `searchSimilar`,
`deleteVector`. The spec's `IHdcPrecompile` only specifies 3:
`storeVector`, `deleteVector`, `searchSimilar`. The extras (`hamming`, `bind`,
`bundle`, `permute`) are used by the broader HDC system but not by
`InsightBoard.sol` directly. This is fine -- the library serves the full
precompile surface.

### F15 -- foundry.toml: Solc Version Mismatch

The `foundry.toml` specifies `solc_version = "0.8.28"` while the pragma in all
Solidity files is `^0.8.20`. This is compatible (0.8.28 satisfies `^0.8.20`),
but the IR pipeline (`via_ir = true`) with optimizer runs 200 is enabled. This
is aggressive optimization; ensure the IR output is tested for correctness,
especially around struct packing.

**File:** `/Users/will/dev/nunchi/daeji/contracts/foundry.toml` lines 6-9

### F16 -- No `receive()` or `fallback()` Function

The contract accepts ETH via `submit()` and `renew()` (both `payable`), and
sends ETH via `purge()`. However, there is no `receive()` or `fallback()`
function. This means the contract cannot receive plain ETH transfers (which is
correct -- it should only accept ETH through defined functions). The remaining
90% of the staked ETH after purge is permanently locked in the contract since
there is no withdrawal mechanism.

**File:** `InsightBoard.sol` (entire contract -- no receive/fallback/withdraw)

**Severity:** Medium. By design, 90% of purged stakes are burned. If this is
intentional (deflationary tokenomics), it should be documented. If not, a
governance-controlled `withdraw()` or redistribution mechanism is needed.

---

## Implementation Status

### Core Contract Functions

| Function | Spec | Implemented | Notes |
|----------|------|-------------|-------|
| `submit()` | Yes | Yes | Signature changed: raw bytes instead of pre-hashed (F03) |
| `confirm()` | Yes | Yes | Correctly accepts 4 states (SUBMITTED, ACTIVE, DECAYING, CHALLENGED) |
| `challenge()` | Yes | Yes | Enhanced: requires anti-knowledge reference (F06), missing resonance check (F07) |
| `computeState()` | Yes | Yes | Correctly uses `lastConfirmedBlock`; differs on non-existent behavior (F08) |
| `renew()` | Yes | Yes | Broken: checks stored state not computed state (F09) |
| `purge()` | Yes | Yes | Works but CEI violation with precompile call (F11), no StateChanged event (F12) |
| `searchSimilar()` | Yes | Yes | Delegated to HdcLib |
| `currentState()` | Yes | No | Spec has a convenience wrapper (line 631); implementation omits it |
| `materializeState()` / `syncState()` | Suggested | No | Critical gap (F09) |

### Enums

| Enum | Spec | Implemented | Match |
|------|------|-------------|-------|
| `Kind` (6 values) | Yes | Yes | Exact match |
| `State` (7 values) | Yes | Yes | Exact match |
| `Tier` (4 values) | Yes | Yes | Exact match |

### Constants

| Constant | Spec | Implemented | Match |
|----------|------|-------------|-------|
| `DUPLICATE_THRESHOLD = 512` | Yes | Yes | Exact |
| `MIN_STAKE = 0.01 ether` | Yes | Yes | Exact |
| `CHALLENGE_RESOLUTION_CONFS = 5` | Inline | Named constant | Improvement |
| `RESONANCE_THRESHOLD = 1024` | N/A | Declared, unused | Dead code (F07) |

### Structs

| Struct | Spec | Implemented | Match |
|--------|------|-------------|-------|
| `InsightAnchor` | Yes | Yes | Exact field layout |
| `Insight` | Yes | Yes | Exact field layout |

### Events

| Event | Spec | Implemented | Notes |
|-------|------|-------------|-------|
| `InsightPublished` | Yes | Yes | Richer signature (F05) |
| `InsightConfirmed` | Yes | Yes | `uint64` vs spec's `uint32` (F05) |
| `InsightChallenged` | Yes | Yes | Includes `challengingInsightId` (F05) |
| `InsightStateChanged` | Yes | Yes | `uint8` instead of `State` enum type |
| `InsightRenewed` | No | Yes | New event (positive) |
| `InsightPurged` | No | Yes | New event (positive) |

### Half-Life Values (blocks)

| Kind | Spec | Implemented | Match |
|------|------|-------------|-------|
| INSIGHT | 648,000 | 648,000 | Yes |
| HEURISTIC | 1,512,000 | 1,512,000 | Yes |
| ANTI_KNOWLEDGE | 3,024,000 | 3,024,000 | Yes |
| WARNING | 432,000 | 432,000 | Yes |
| CAUSAL_LINK | 2,160,000 | 2,160,000 | Yes |
| STRATEGY | 1,080,000 | 1,080,000 | Yes |

### Tier Multipliers

| Tier | Spec | Implemented | Match |
|------|------|-------------|-------|
| TRANSIENT | 1 | 1 | Yes |
| WORKING | 3 | 3 | Yes |
| CONSOLIDATED | 7 | 7 | Yes |
| PERSISTENT | 10 | 10 | Yes |

---

## Anti-Patterns & Duct Tape

### AP01 -- TestableInsightBoard Duplicates Core Logic

**File:** `/Users/will/dev/nunchi/daeji/contracts/test/InsightBoard.t.sol` lines 11-90

The `TestableInsightBoard` test contract overrides `submit()` and `purge()` by
copy-pasting their entire function bodies and removing the precompile calls.
This is classic duct tape: if the real `submit()` or `purge()` changes, the
test override must be manually synchronized. Any bug fix or feature addition
to the real contract must be duplicated in the test mock.

**Better approach:** Use Foundry's `vm.mockCall()` or `vm.etch()` to mock the
precompile at address `0x09`, then test the real `InsightBoard` contract
directly. Example:

```solidity
// Mock the precompile to return empty results for searchSimilar
vm.mockCall(
    address(0x09),
    abi.encodePacked(uint8(0x02)), // searchSimilar selector
    abi.encode(new bytes32[](0), new uint16[](0))
);
```

### AP02 -- Duplicate Check Skipped in Tests

**File:** `/Users/will/dev/nunchi/daeji/contracts/test/InsightBoard.t.sol` line 24

The `TestableInsightBoard.submit()` override skips the duplicate check entirely
(`// Skip duplicate check (no precompile in test)`). This means the
`DUPLICATE_THRESHOLD` constant and the duplicate detection logic on
`InsightBoard.sol` lines 79-86 are completely untested. Any regression in
duplicate checking will not be caught.

### AP03 -- Dead Constant: RESONANCE_THRESHOLD

**File:** `/Users/will/dev/nunchi/daeji/contracts/src/InsightBoard.sol` line 27

`RESONANCE_THRESHOLD = 1024` is declared but never referenced in any function.
It appears to be intended for the `challenge()` resonance check (F07) that was
never implemented. Dead code should be removed or the feature should be
completed.

### AP04 -- Stored vs Computed State Inconsistency

The contract has two sources of truth for state: `insight.anchor.state` (stored)
and `computeState(insightId)` (computed). Some functions check stored state
(`confirm()` line 154, `renew()` line 245, `challenge()` line 215), while
`purge()` checks computed state (line 271). This inconsistency means:

- `renew()` is unreachable for naturally-decayed insights (F09)
- `confirm()` does not see time-based state changes (F10)
- Event emissions are inaccurate for time-based transitions

This is the single most impactful design problem in the contract. The spec
acknowledges it (lines 1249-1255) but the implementation does not resolve it.

### AP05 -- InsightView Struct Defined in Test Only

**File:** `/Users/will/dev/nunchi/daeji/contracts/test/InsightBoard.t.sol` lines 98-110

The test defines `InsightView` to destructure the auto-generated `insights()`
getter return. This is fragile -- if the `Insight` struct changes, the test
struct must be updated manually. Consider adding a proper getter function to
the contract or interface.

---

## Security Concerns

### SEC01 -- Reentrancy in purge() [Low]

**File:** `InsightBoard.sol` lines 263-293

The `purge()` function performs an external call to the precompile
(`HdcLib.deleteVector()` on line 280) before deleting storage (line 283) and
before the ETH transfer (line 290). The precompile at `0x09` is trusted
infrastructure, but the ordering violates strict CEI. The ETH transfer to the
author on line 290 is also a reentrancy vector. The `delete insights[insightId]`
on line 283 prevents re-purging the same ID, which is sufficient for
same-function reentrancy. Cross-function reentrancy is not a risk because no
other function can act on a deleted insight.

**Recommendation:** Add `ReentrancyGuard` (OpenZeppelin) as defense-in-depth.
Move `delete insights[insightId]` before `HdcLib.deleteVector()` for stricter
CEI compliance.

### SEC02 -- No Access Control on purge() [Low]

Anyone can call `purge()` on any purgeable insight. The 10% legacy goes to
the original author, and the purger gets SSTORE refunds. This is by design
(incentivizing garbage collection), but it means anyone can force-purge as
soon as the 10x half-life expires.

### SEC03 -- Front-Running on submit() [Medium]

**File:** `InsightBoard.sol` lines 68-135

The full vector and content are passed as calldata and visible in the mempool.
An attacker can front-run by submitting the same vector first (claiming
authorship and the stake). The hash-based ID includes `msg.sender` and
`block.number`, so the attacker gets a different ID, but the duplicate check
(lines 79-86) would then reject the original author's submission.

**Recommendation:** Consider commit-reveal for high-value insights, or accept
the risk if the chain uses a private/sequenced mempool.

### SEC04 -- Griefing via Unrelated Anti-Knowledge Challenges [Medium]

**File:** `InsightBoard.sol` lines 197-234

As noted in F07, the `challenge()` function verifies that the challenging
entry has `kind == ANTI_KNOWLEDGE` but does NOT verify vector similarity
between the target and the anti-knowledge. An attacker can publish a
semantically unrelated `ANTI_KNOWLEDGE` entry and use it to challenge any
`ACTIVE` insight. The cost is `MIN_STAKE` (0.01 ETH) for the anti-knowledge
entry, and the target enters `CHALLENGED` state requiring 5 new confirmations
to recover.

### SEC05 -- Staked ETH Permanently Locked [Medium]

**File:** `InsightBoard.sol` (entire contract)

When `purge()` is called, only 10% of the staked amount is returned to the
author. The remaining 90% stays in the contract forever. There is no
`withdraw()`, no governance mechanism, and no `receive()`/`fallback()`. Over
time, the contract will accumulate ETH that cannot be retrieved.

If this is intentional (burn mechanism), it should be documented. If not,
consider:
- Sending remaining 90% to a treasury or DAO address
- Distributing to recent confirmers as a reward
- Burning by sending to `address(0)`

### SEC06 -- No Rate Limiting on submit() [Low]

The spec mentions (line 808): "Consider rate-limiting submissions per address
per block." The implementation does not implement any rate limiting. An attacker
with sufficient ETH can spam submissions at `MIN_STAKE` each.

### SEC07 -- Author Can Confirm Own Insight [Low]

**File:** `InsightBoard.sol` lines 139-194

There is no check preventing `msg.sender == insight.anchor.author`. The author
of an insight can also be its first confirmer, immediately moving it from
`SUBMITTED` to `ACTIVE`. This is a low-severity concern because each unique
address can only confirm once, but it reduces the effective "independent
validation" count by 1.

---

## Recommended Changes Checklist

### Critical (Blocks core functionality)

- [ ] **Add `materializeState(bytes32 insightId)` function** that writes
      `computeState(insightId)` back to `insight.anchor.state`. Without this,
      `renew()` is unreachable for naturally-decayed insights. This function
      should be callable by anyone and should emit `InsightStateChanged`.
      (`InsightBoard.sol`, new function)
- [ ] **OR: Change `renew()` to use `computeState()`** instead of checking
      stored `insight.anchor.state`. Same for `confirm()` state checks.
      (`InsightBoard.sol` lines 154-161, 244-245)

### High (Correctness / spec compliance)

- [ ] **Implement resonance check in `challenge()`** using
      `HdcLib.searchSimilar()` or `HdcLib.hamming()` to verify the
      anti-knowledge vector is semantically related to the target. Use the
      existing `RESONANCE_THRESHOLD` constant (1024 Hamming distance = 90%
      similarity). (`InsightBoard.sol` lines 197-234)
- [ ] **Remove or use `RESONANCE_THRESHOLD`** -- dead code on line 27.
      Either implement the resonance check or delete the constant.
- [ ] **Fix test mock pattern** -- replace `TestableInsightBoard` copy-paste
      overrides with `vm.mockCall()` / `vm.etch()` so the real contract logic
      is tested. (`InsightBoard.t.sol` lines 11-90)

### Medium (Security hardening)

- [ ] **Add `ReentrancyGuard`** (OpenZeppelin) to `purge()`.
      (`InsightBoard.sol` line 263)
- [ ] **Move `delete insights[insightId]` before `HdcLib.deleteVector()`**
      in `purge()` for strict CEI compliance.
      (`InsightBoard.sol` lines 280-283 -- swap order)
- [ ] **Document or resolve the 90% ETH lockup** in `purge()`. If intentional,
      add a NatSpec comment. If not, add a treasury address or burn mechanism.
      (`InsightBoard.sol` lines 277, 289-291)
- [ ] **Add author-cannot-self-confirm check** to `confirm()`:
      `require(msg.sender != insight.anchor.author)`.
      (`InsightBoard.sol` line 139)

### Low (Completeness / polish)

- [ ] **Emit `InsightStateChanged` in `purge()`** to maintain a complete
      event log of all state transitions. (`InsightBoard.sol` after line 286)
- [ ] **Add `currentState()` convenience function** as defined in the spec
      (line 631). (`InsightBoard.sol`, new function)
- [ ] **Add tests for `renew()` happy path** -- the current test only covers
      the revert case (`test_renew_nonArchivedReverts`). No test confirms
      successful renewal. (`InsightBoard.t.sol`)
- [ ] **Add test for DECAYING -> ACTIVE via `confirm()`** -- while this path
      is implicitly tested via `test_computeState_usesLastConfirmedBlock`, there
      is no explicit test that calls `confirm()` on an insight whose stored
      state is `DECAYING` (requires `materializeState()` or manual storage
      manipulation). (`InsightBoard.t.sol`)
- [ ] **Add test for duplicate vector detection** -- currently untested because
      `TestableInsightBoard` skips the precompile. (`InsightBoard.t.sol`)
- [ ] **Add test for event emissions on tier promotion** -- no test verifies
      that tier changes emit appropriate events (they currently do not -- tier
      changes are silent). Consider adding a `TierPromoted` event.
      (`InsightBoard.t.sol`, `IInsightBoard.sol`)
- [ ] **Add fuzz tests** for `computeState()` boundary conditions (e.g., age
      exactly at 1x, 2x, 5x, 10x half-life thresholds).
      (`InsightBoard.t.sol`)

---

## Second-Pass Remediation Detail

This pass was performed against the current implementation files:

- `contracts/src/InsightBoard.sol`
- `contracts/src/IInsightBoard.sol`
- `contracts/src/HdcPrecompile.sol`
- `contracts/test/InsightBoard.t.sol`

The fixes below are implementation instructions only. They should be applied in
Solidity in a follow-up code change, with tests updated in the same commit.

### 1. Stored vs Computed State

**Current problem:** `computeState()` is the only place where time decay is
applied, but `confirm()`, `challenge()`, and `renew()` mostly gate on
`insight.anchor.state`. This makes stored state stale and breaks natural
ARCHIVED renewal.

**Concrete fix:** introduce a single state materialization path and use it at
the start of every write function that cares about lifecycle state.

```solidity
function _materializeState(bytes32 insightId)
    internal
    returns (Insight storage insight, State current)
{
    insight = insights[insightId];
    require(insight.anchor.publishBlock != 0, "InsightBoard: insight does not exist");

    current = computeState(insightId);
    State stored = State(insight.anchor.state);

    if (current != stored) {
        insight.anchor.state = uint8(current);
        emit InsightStateChanged(insightId, uint8(stored), uint8(current));
    }
}
```

Rules after this change:

- `computeState()` remains the read-time oracle for decay.
- `_materializeState()` is the only write path that copies computed state into
  storage.
- `confirm()`, `challenge()`, `renew()`, and `purge()` must use the materialized
  `current` state instead of reading `insight.anchor.state` directly.
- If a write function transitions state after materialization, emit a second
  `InsightStateChanged` for the explicit transition. This preserves a complete
  event trail: stale `ACTIVE -> DECAYING`, then `DECAYING -> ACTIVE`, for
  example.

### 2. confirm(), renew(), and purge() Semantics

**`confirm()`**

Use `_materializeState()` before the confirmer check. Permit only
`SUBMITTED`, `ACTIVE`, `DECAYING`, and `CHALLENGED`. Reject `ARCHIVED` and
`PURGED`; `ARCHIVED` must go through `renew()`, and `PURGED` must go through
`purge()`.

```solidity
(Insight storage insight, State current) = _materializeState(insightId);
require(current != State.ARCHIVED, "InsightBoard: archived; renew required");
require(current != State.PURGED, "InsightBoard: purged");
require(
    current == State.SUBMITTED ||
    current == State.ACTIVE ||
    current == State.DECAYING ||
    current == State.CHALLENGED,
    "InsightBoard: not confirmable"
);
```

When `current == State.DECAYING`, `confirm()` should transition to `ACTIVE`,
reset `lastConfirmedBlock`, reset `confirmsSinceChallenge`, and emit
`InsightStateChanged(insightId, DECAYING, ACTIVE)`. When `current ==
State.ACTIVE`, it only refreshes `lastConfirmedBlock`.

**`renew()`**

Change the guard from stored `insight.anchor.state == ARCHIVED` to
materialized/computed `current == ARCHIVED`. This is the fix that makes renewal
reachable for naturally decayed insights.

```solidity
(Insight storage insight, State current) = _materializeState(insightId);
require(current == State.ARCHIVED, "InsightBoard: only ARCHIVED insights can be renewed");
require(msg.value >= MIN_STAKE, "InsightBoard: insufficient stake");

insight.anchor.publishBlock = uint64(block.number);
insight.lastConfirmedBlock = uint64(block.number);
insight.confirmsSinceChallenge = 0;
insight.stakedAmount += msg.value;
insight.anchor.state = uint8(State.ACTIVE);

emit InsightRenewed(insightId, msg.sender, msg.value);
emit InsightStateChanged(insightId, uint8(State.ARCHIVED), uint8(State.ACTIVE));
```

Preserve `confirmations` and `tier` unless product requirements explicitly say
renewal should reset reputation. The current lifecycle reads as renewal, not a
new insight, so preserving them is the least surprising behavior.

**`purge()`**

Keep the open garbage-collection model, but enforce strict checks-effects-
interactions. Cache the author and stake, delete storage and credit withdrawals
before any external call, emit the purge events, then interact with the HDC
precompile.

```solidity
(Insight storage insight, State current) = _materializeState(insightId);
require(current == State.PURGED, "InsightBoard: not yet purgeable");

address author = insight.anchor.author;
uint256 stake = insight.stakedAmount;

delete insights[insightId];
_creditPurgeStake(author, stake);

// Terminal deletion marker. The stale-state materialization, if any, has
// already emitted oldStored -> PURGED before deletion.
emit InsightStateChanged(insightId, uint8(State.PURGED), uint8(State.PURGED));
emit InsightPurged(insightId, msg.sender, author, stake);

HdcLib.deleteVector(insightId);
```

If `HdcLib.deleteVector()` fails, the transaction reverts and the prior storage
delete and withdrawal credits are reverted too. The important improvement is
that no reentrant path sees the target insight as live after checks have passed,
and no accounting mutation happens after an external call.

### 3. Stake Accounting and Withdrawals

**Current problem:** `submit()` and `renew()` accept native token stake,
`purge()` immediately pushes 10% to the author, and the remaining 90% is left
inside the contract with no owner, withdrawal path, or explicit burn semantics.

**Concrete fix:** define stake policy explicitly and switch payouts to pull
withdrawals.

Recommended accounting:

```solidity
uint16 public constant AUTHOR_LEGACY_BPS = 1_000; // 10%
uint16 public constant TREASURY_BPS = 9_000;      // 90%
uint16 public constant BPS_DENOMINATOR = 10_000;

address public immutable treasury;
mapping(address => uint256) public pendingWithdrawals;
```

Purge should credit balances, not push ETH:

```solidity
function _creditPurgeStake(address author, uint256 stake) internal {
    uint256 authorAmount = (stake * AUTHOR_LEGACY_BPS) / BPS_DENOMINATOR;
    uint256 treasuryAmount = stake - authorAmount;

    pendingWithdrawals[author] += authorAmount;
    pendingWithdrawals[treasury] += treasuryAmount;

    emit StakeCredited(author, authorAmount, StakeCreditReason.Legacy);
    emit StakeCredited(treasury, treasuryAmount, StakeCreditReason.Treasury);
}
```

With OpenZeppelin Contracts v5, import `ReentrancyGuard` from `utils`:

```solidity
import {ReentrancyGuard} from "@openzeppelin/contracts/utils/ReentrancyGuard.sol";

contract InsightBoard is IInsightBoard, ReentrancyGuard {
    // ...
}
```

Withdraw using CEI and `nonReentrant`:

```solidity
function withdraw() external nonReentrant {
    uint256 amount = pendingWithdrawals[msg.sender];
    require(amount != 0, "InsightBoard: nothing to withdraw");

    pendingWithdrawals[msg.sender] = 0;

    (bool ok, ) = payable(msg.sender).call{value: amount}("");
    require(ok, "InsightBoard: withdrawal failed");

    emit Withdrawal(msg.sender, amount);
}
```

Also choose one overpayment policy for `submit()` and `renew()`:

- strict: `require(msg.value == MIN_STAKE, "InsightBoard: exact stake required")`
- flexible: account the full `msg.value` and make all of it subject to the purge
  split
- refund: credit `msg.value - MIN_STAKE` to `pendingWithdrawals[msg.sender]`

Do not leave excess value or the 90% purge remainder as undocumented stranded
contract balance.

### 4. Challenge Resonance

**Current problem:** `challenge()` requires an `ANTI_KNOWLEDGE` insight but does
not prove semantic relationship. `RESONANCE_THRESHOLD = 1024` is declared and
unused.

**Concrete fix:** require all of the following:

- `targetInsightId != antiKnowledgeId`
- `computeState(targetInsightId) == ACTIVE` after materialization
- `anti.anchor.kind == uint8(Kind.ANTI_KNOWLEDGE)`
- optional but recommended: `anti.anchor.author == msg.sender`, so challengers
  cannot reuse another author's anti-knowledge entry
- HDC distance is below `RESONANCE_THRESHOLD`

The current `HdcLib.hamming(bytes,bytes)` only works with raw vectors, while the
contract stores only `vectorHash`. Choose one of these implementation paths:

1. **Preferred precompile API fix:** add a by-id HDC distance operation to the
   precompile, for example `hammingById(bytes32 a, bytes32 b)`, and call it in
   `challenge()`.
2. **No precompile API change:** extend `challenge()` to accept both raw vectors,
   verify `keccak256(targetVector) == target.anchor.vectorHash` and
   `keccak256(antiVector) == anti.anchor.vectorHash`, then call
   `HdcLib.hamming(targetVector, antiVector)`.

The guard should be strict enough to match the constant comment:

```solidity
uint32 distance = HdcLib.hamming(targetVector, antiVector);
require(distance < RESONANCE_THRESHOLD, "InsightBoard: insufficient resonance");
```

If neither path is implemented, delete `RESONANCE_THRESHOLD` and document that
challenge resonance is enforced off-chain. Keeping the unused constant implies a
security property the contract does not currently provide.

### 5. Self-Confirm Guard

**Current problem:** an author can confirm their own insight and move it from
`SUBMITTED` to `ACTIVE`, which weakens the "independent validation" model.

**Concrete fix:** after existence/materialization and before writing to
`confirmers`, add:

```solidity
require(msg.sender != insight.anchor.author, "InsightBoard: author cannot confirm");
```

Add this before `confirmers[insightId][msg.sender] = true` so a reverted
self-confirm does not poison the confirmer mapping.

### 6. Event Schema

**Current problem:** events are richer than the original spec, but purge does
not emit `InsightStateChanged`, stake movements are not observable, and renewal
does not include the added stake. The interface and tests should lock the final
event schema.

**Concrete fix:** keep the `bytes32`-based schema and add amount/account fields
where indexers need accounting:

```solidity
event InsightRenewed(
    bytes32 indexed insightId,
    address indexed renewer,
    uint256 addedStake
);

event InsightPurged(
    bytes32 indexed insightId,
    address indexed purger,
    address indexed author,
    uint256 stakedAmount
);

event StakeCredited(
    address indexed account,
    uint256 amount,
    StakeCreditReason reason
);

event Withdrawal(address indexed account, uint256 amount);
```

For state history:

- Emit `InsightStateChanged` whenever `_materializeState()` changes stored
  state.
- Emit `InsightStateChanged` for explicit transitions caused by `confirm()`,
  `challenge()`, `renew()`, and `purge()`.
- Keep `InsightPublished` as the only event carrying full `vector` and `content`
  if event-log reconstruction of the HDC index remains a requirement. If not,
  replace the content bytes with `contentHash` plus an off-chain URI to reduce
  LOG data cost.

### 7. Testable Contract Duplication

**Current problem:** `contracts/test/InsightBoard.t.sol` defines
`TestableInsightBoard` that copy-pastes `submit()` and `purge()` and removes
precompile calls. This can drift from production logic.

**Concrete fix:** test the real `InsightBoard` and mock the precompile boundary.

Recommended Foundry approach:

- Deploy `InsightBoard` directly in `setUp()`.
- Use `vm.etch(address(0x09), mockCode)` for an in-test selector-dispatch mock
  that implements `storeVector`, `searchSimilar`, `deleteVector`, and `hamming`.
- Use `vm.mockCall()` only for simple static responses. Prefer `vm.etch()` once
  tests need stateful behavior such as duplicate detection and delete tracking.
- Delete `TestableInsightBoard` after the precompile mock is in place.

The mock should expose assertions or public state for:

- stored vector ids
- deleted vector ids
- configurable duplicate search results
- configurable hamming distances for resonance tests

### 8. Foundry Tests to Add

Add targeted tests before refactoring large surfaces. Minimum set:

```solidity
function test_confirm_authorCannotSelfConfirm() public;
function test_confirm_decayingComputedStateMaterializesThenRecovers() public;
function test_confirm_archivedComputedStateRevertsRenewRequired() public;
function test_confirm_purgedComputedStateReverts() public;

function test_renew_computedArchivedToActive() public;
function test_renew_revertsIfComputedPurged() public;
function test_renew_addsStakeAndResetsBlocks() public;
function test_renew_emitsRenewedAndStateChanged() public;

function test_purge_deletesStorageBeforeExternalPayoutPath() public;
function test_purge_creditsAuthorAndTreasuryWithdrawals() public;
function test_purge_deletesVectorFromPrecompile() public;
function test_withdraw_usesPendingBalanceAndZeroesBeforeCall() public;
function test_withdraw_reentrancyBlocked() public;

function test_challenge_revertsWhenAntiKnowledgeUnrelated() public;
function test_challenge_acceptsResonantAntiKnowledge() public;
function test_challenge_revertsSelfChallenge() public;
function test_challenge_revertsWhenAntiKnowledgeNotAuthoredByCaller() public;

function test_submit_duplicateVectorRevertsViaPrecompileMock() public;
function test_events_stateChangedOnMaterializeRenewPurge() public;
function testFuzz_computeState_boundaries(uint64 ageOffset) public;
```

Boundary tests should cover exact thresholds and just-over thresholds:

- `age == 1x`, `age == 1x + 1`
- `age == 2x`, `age == 2x + 1` for `CHALLENGED`
- `age == 5x`, `age == 5x + 1`
- `age == 10x`, `age == 10x + 1`

### 9. External References

- Solidity Common Patterns, "Withdrawal from Contracts":
  <https://docs.soliditylang.org/en/latest/common-patterns.html#withdrawal-from-contracts>
- Solidity Security Considerations, "Use the Checks-Effects-Interactions Pattern":
  <https://docs.soliditylang.org/en/latest/security-considerations.html#use-the-checks-effects-interactions-pattern>
- OpenZeppelin Contracts v5 API, `ReentrancyGuard` under `utils`:
  <https://docs.openzeppelin.com/contracts/5.x/api/utils#ReentrancyGuard>
