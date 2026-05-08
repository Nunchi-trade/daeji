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

    // --- Stake accounting events ---

    event StakeDeposited(bytes32 indexed insightId, address indexed author, uint256 amount);
    event StakeSlashed(bytes32 indexed insightId, address indexed challenger, uint256 slashedAmount, uint256 challengerReward);
    event StakeUnlocked(bytes32 indexed insightId, address indexed author, uint256 unlockedAmount, uint8 newTier);
    event WithdrawalClaimed(address indexed recipient, uint256 amount);
    event ProtocolFeeCollected(bytes32 indexed insightId, uint256 amount);

    // ---------------------------------------------------------------
    // Write functions
    // ---------------------------------------------------------------

    /// @notice Submit a new insight with stake.
    function submit(
        Kind kind,
        bytes calldata vector,
        bytes calldata content
    ) external payable returns (bytes32 insightId);

    /// @notice Confirm an existing insight.
    function confirm(bytes32 insightId) external;

    /// @notice Challenge an ACTIVE insight by referencing anti-knowledge.
    function challenge(bytes32 targetInsightId, bytes32 antiKnowledgeId) external;

    /// @notice Renew an ARCHIVED insight. Requires fresh stake >= MIN_STAKE.
    function renew(bytes32 insightId) external payable;

    /// @notice Purge a PURGED insight. Clears storage, credits partial stake
    ///         to author and cleanup caller via pendingWithdrawals.
    function purge(bytes32 insightId) external;

    /// @notice Withdraw accumulated pending withdrawals (pull pattern).
    function withdraw() external;

    // ---------------------------------------------------------------
    // View functions
    // ---------------------------------------------------------------

    /// @notice Compute the current lifecycle state of an insight.
    function computeState(bytes32 insightId) external view returns (State);

    /// @notice Search for similar vectors via the HDC precompile.
    function searchSimilar(
        bytes calldata queryVector,
        uint8 topK
    ) external view returns (bytes32[] memory ids, uint16[] memory distances);
}
