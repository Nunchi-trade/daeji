// SPDX-License-Identifier: MIT
pragma solidity ^0.8.20;

import {IInsightBoard} from "./IInsightBoard.sol";

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
    uint16 public constant RESONANCE_THRESHOLD = 1024;

    /// @dev Protocol fee address receives 50% of slashed stake.
    address public constant PROTOCOL_FEE_ADDRESS = address(0xFEE);

    // ---------------------------------------------------------------
    // Structs
    // ---------------------------------------------------------------

    /// @dev On-chain anchor for an insight. Packed into 3 storage slots.
    struct InsightAnchor {
        bytes32 vectorHash;      // slot 0
        bytes32 contentHash;     // slot 1
        address author;          // slot 2: 20 bytes
        uint64  publishBlock;    //          8 bytes
        uint8   kind;            //          1 byte
        uint8   tier;            //          1 byte
        uint8   state;           //          1 byte
    }

    struct Insight {
        InsightAnchor anchor;           // 3 storage slots
        uint64 confirmations;           // slot 3 (packed)
        uint64 lastConfirmedBlock;      //
        uint64 confirmsSinceChallenge;  //
        uint256 stakedAmount;           // slot 4
        uint256 originalStake;          // slot 5: original stake for unlock calculations
        uint256 unlockedAmount;         // slot 6: total already unlocked
    }

    // ---------------------------------------------------------------
    // Storage
    // ---------------------------------------------------------------

    /// @notice All insights by their unique ID.
    mapping(bytes32 => Insight) public insights;

    /// @notice Tracks which addresses have confirmed each insight.
    mapping(bytes32 => mapping(address => bool)) public confirmers;

    /// @notice Pull-withdrawal balances.
    mapping(address => uint256) public pendingWithdrawals;

    // ---------------------------------------------------------------
    // Write Functions
    // ---------------------------------------------------------------

    /// @inheritdoc IInsightBoard
    /// @dev Duplicate detection: The v1 precompile does not have searchSimilar.
    ///      The full vector is emitted in InsightPublished so off-chain indexers
    ///      can detect near-duplicates using HdcLib.isSimilar (opcode 0x06).
    function submit(
        Kind kind,
        bytes calldata vector,
        bytes calldata content
    ) external payable virtual returns (bytes32 insightId) {
        require(vector.length == 1280, "InsightBoard: invalid vector size");
        require(msg.value >= MIN_STAKE, "InsightBoard: insufficient stake");

        bytes32 vectorHash = keccak256(vector);

        // Compute unique ID
        insightId = keccak256(
            abi.encodePacked(vectorHash, msg.sender, block.number)
        );

        // Existence guard
        require(
            insights[insightId].anchor.publishBlock == 0,
            "InsightBoard: ID collision"
        );

        // Write state (CEI: state changes BEFORE external calls)
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
            stakedAmount: msg.value,
            originalStake: msg.value,
            unlockedAmount: 0
        });

        // Emit events AFTER state changes (CEI pattern)
        // Vector emitted here for off-chain duplicate detection via isSimilar
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

        emit StakeDeposited(insightId, msg.sender, msg.value);
    }

    /// @inheritdoc IInsightBoard
    /// @dev CRITICAL: Must accept 4 states (SUBMITTED, ACTIVE, DECAYING, CHALLENGED).
    function confirm(bytes32 insightId) external {
        require(
            !confirmers[insightId][msg.sender],
            "InsightBoard: already confirmed"
        );

        Insight storage insight = insights[insightId];

        // Existence check
        require(
            insight.anchor.publishBlock != 0,
            "InsightBoard: insight does not exist"
        );

        // State check: accept 4 confirmable states
        uint8 currentState = insight.anchor.state;
        require(
            currentState == uint8(State.SUBMITTED) ||
            currentState == uint8(State.ACTIVE) ||
            currentState == uint8(State.DECAYING) ||
            currentState == uint8(State.CHALLENGED),
            "InsightBoard: not confirmable"
        );

        // Update confirmation data
        confirmers[insightId][msg.sender] = true;
        insight.confirmations++;
        insight.lastConfirmedBlock = uint64(block.number);

        // State transitions on confirmation
        uint8 oldState = currentState;
        if (
            currentState == uint8(State.SUBMITTED) ||
            currentState == uint8(State.DECAYING)
        ) {
            insight.anchor.state = uint8(State.ACTIVE);
            insight.confirmsSinceChallenge = 0;
        } else if (currentState == uint8(State.CHALLENGED)) {
            insight.confirmsSinceChallenge++;
            if (insight.confirmsSinceChallenge >= CHALLENGE_RESOLUTION_CONFS) {
                insight.anchor.state = uint8(State.ACTIVE);
                insight.confirmsSinceChallenge = 0;
            }
        }
        // State.ACTIVE: no state change, just refresh lastConfirmedBlock.

        // Tier promotion based on total confirmations (with stake unlock)
        _promoteTier(insightId, insight);

        // Emit events AFTER all state changes (CEI)
        emit InsightConfirmed(insightId, msg.sender, insight.confirmations);

        if (insight.anchor.state != oldState) {
            emit InsightStateChanged(insightId, oldState, insight.anchor.state);
        }
    }

    /// @inheritdoc IInsightBoard
    function challenge(
        bytes32 targetInsightId,
        bytes32 antiKnowledgeId
    ) external {
        Insight storage target = insights[targetInsightId];
        Insight storage anti = insights[antiKnowledgeId];

        // Existence checks
        require(
            target.anchor.publishBlock != 0,
            "InsightBoard: target does not exist"
        );
        require(
            anti.anchor.publishBlock != 0,
            "InsightBoard: anti-knowledge does not exist"
        );

        // Target must be ACTIVE
        require(
            target.anchor.state == uint8(State.ACTIVE),
            "InsightBoard: target not ACTIVE"
        );

        // Anti-knowledge must be of kind ANTI_KNOWLEDGE
        require(
            anti.anchor.kind == uint8(Kind.ANTI_KNOWLEDGE),
            "InsightBoard: challenger is not ANTI_KNOWLEDGE"
        );

        // Slash 50% of remaining stake
        uint256 remaining = target.stakedAmount;
        uint256 slashAmount = remaining / 2;
        uint256 challengerReward = slashAmount / 2;
        uint256 protocolFee = slashAmount - challengerReward;

        target.stakedAmount = remaining - slashAmount;

        // Credit challenger and protocol fee address via pull pattern
        if (challengerReward > 0) {
            pendingWithdrawals[msg.sender] += challengerReward;
        }
        if (protocolFee > 0) {
            pendingWithdrawals[PROTOCOL_FEE_ADDRESS] += protocolFee;
        }

        // State transition: ACTIVE -> CHALLENGED
        uint8 oldState = target.anchor.state;
        target.anchor.state = uint8(State.CHALLENGED);
        target.confirmsSinceChallenge = 0;

        // Emit events AFTER state changes (CEI)
        emit StakeSlashed(targetInsightId, msg.sender, slashAmount, challengerReward);
        if (protocolFee > 0) {
            emit ProtocolFeeCollected(targetInsightId, protocolFee);
        }
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

        emit StakeDeposited(insightId, insight.anchor.author, msg.value);
        emit InsightRenewed(insightId, msg.sender);
        emit InsightStateChanged(insightId, oldState, uint8(State.ACTIVE));
    }

    /// @inheritdoc IInsightBoard
    /// @dev Pull-withdrawal pattern: credits pendingWithdrawals instead of
    ///      direct ETH transfer. Cleanup caller gets 5%, author gets 10%.
    function purge(bytes32 insightId) external virtual {
        Insight storage insight = insights[insightId];

        require(
            insight.anchor.publishBlock != 0,
            "InsightBoard: insight does not exist"
        );
        require(
            computeState(insightId) == State.PURGED,
            "InsightBoard: not yet purgeable"
        );

        // Cache values before deletion
        address author = insight.anchor.author;
        uint256 staked = insight.stakedAmount;
        uint256 authorLegacy = staked / 10;       // 10% to original author
        uint256 cleanupReward = staked / 20;       // 5% to cleanup caller

        // Credit via pull pattern
        if (authorLegacy > 0) {
            pendingWithdrawals[author] += authorLegacy;
        }
        if (cleanupReward > 0) {
            pendingWithdrawals[msg.sender] += cleanupReward;
        }

        // Clear storage (triggers SSTORE refund)
        delete insights[insightId];

        emit InsightPurged(insightId, msg.sender);
    }

    /// @inheritdoc IInsightBoard
    function withdraw() external {
        uint256 amount = pendingWithdrawals[msg.sender];
        require(amount > 0, "InsightBoard: nothing to withdraw");

        pendingWithdrawals[msg.sender] = 0;

        emit WithdrawalClaimed(msg.sender, amount);

        (bool ok, ) = msg.sender.call{value: amount}("");
        require(ok, "InsightBoard: withdrawal failed");
    }

    // ---------------------------------------------------------------
    // View Functions
    // ---------------------------------------------------------------

    /// @inheritdoc IInsightBoard
    /// @dev CRITICAL: Uses age relative to lastConfirmedBlock (NOT publishBlock).
    function computeState(bytes32 insightId) public view returns (State) {
        Insight storage insight = insights[insightId];

        // Non-existent insights return PURGED
        if (insight.anchor.publishBlock == 0) {
            return State.PURGED;
        }

        // Age computation from lastConfirmedBlock
        uint64 age = uint64(block.number) - insight.lastConfirmedBlock;
        uint64 tierMult = _tierMultiplier(Tier(insight.anchor.tier));
        uint64 kindHL = _kindHalfLife(Kind(insight.anchor.kind));
        uint64 effectiveHL = kindHL * tierMult;

        // Decay thresholds take precedence over stored state
        if (age > effectiveHL * 10) {
            return State.PURGED;
        }

        // CHALLENGED entries use 2x half-life for ARCHIVED threshold
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

        // Within active lifetime: preserve stored state
        return State(insight.anchor.state);
    }

    /// @inheritdoc IInsightBoard
    /// @dev searchSimilar is not available in v1 precompile. Returns empty
    ///      arrays. Off-chain indexers should use emitted vectors + isSimilar.
    function searchSimilar(
        bytes calldata,
        uint8
    ) external pure returns (bytes32[] memory ids, uint16[] memory distances) {
        ids = new bytes32[](0);
        distances = new uint16[](0);
    }

    // ---------------------------------------------------------------
    // Internal Helpers
    // ---------------------------------------------------------------

    /// @dev Kind-specific base half-lives in blocks (~400ms/block).
    function _kindHalfLife(Kind kind) internal pure returns (uint64) {
        if (kind == Kind.INSIGHT)        return  10_000;
        if (kind == Kind.HEURISTIC)      return  15_000;
        if (kind == Kind.ANTI_KNOWLEDGE) return   5_000;
        if (kind == Kind.WARNING)        return   3_000;
        if (kind == Kind.CAUSAL_LINK)    return  20_000;
        if (kind == Kind.STRATEGY)       return   8_000;
        revert("InsightBoard: unknown kind");
    }

    /// @dev Tier multipliers for effective half-life.
    function _tierMultiplier(Tier tier) internal pure returns (uint64) {
        if (tier == Tier.TRANSIENT)    return 1;
        if (tier == Tier.WORKING)      return 3;
        if (tier == Tier.CONSOLIDATED) return 7;
        if (tier == Tier.PERSISTENT)   return 10;
        revert("InsightBoard: unknown tier");
    }

    /// @dev Promote tier based on total confirmation count.
    ///      Promotions are monotonic (never demote).
    ///      On promotion: unlock partial stake to author's pendingWithdrawals.
    ///      - WORKING (3 confs): 20% of original stake
    ///      - CONSOLIDATED (10 confs): 40% of original stake (cumulative)
    ///      - PERSISTENT (25 confs): 60% of original stake (cumulative)
    function _promoteTier(bytes32 insightId, Insight storage insight) internal {
        uint64 confs = insight.confirmations;
        uint8 oldTier = insight.anchor.tier;
        uint8 newTier = oldTier;

        if (confs >= 25 && oldTier < uint8(Tier.PERSISTENT)) {
            newTier = uint8(Tier.PERSISTENT);
        } else if (confs >= 10 && oldTier < uint8(Tier.CONSOLIDATED)) {
            newTier = uint8(Tier.CONSOLIDATED);
        } else if (confs >= 3 && oldTier < uint8(Tier.WORKING)) {
            newTier = uint8(Tier.WORKING);
        }

        if (newTier != oldTier) {
            insight.anchor.tier = newTier;

            // Calculate cumulative unlock percentage for new tier
            uint256 unlockPercent;
            if (newTier == uint8(Tier.WORKING))      unlockPercent = 20;
            else if (newTier == uint8(Tier.CONSOLIDATED)) unlockPercent = 40;
            else if (newTier == uint8(Tier.PERSISTENT))   unlockPercent = 60;

            uint256 totalUnlock = (insight.originalStake * unlockPercent) / 100;
            uint256 newUnlock = totalUnlock - insight.unlockedAmount;

            if (newUnlock > 0 && newUnlock <= insight.stakedAmount) {
                insight.unlockedAmount = totalUnlock;
                insight.stakedAmount -= newUnlock;
                pendingWithdrawals[insight.anchor.author] += newUnlock;

                emit StakeUnlocked(insightId, insight.anchor.author, newUnlock, newTier);
            }
        }
    }
}
