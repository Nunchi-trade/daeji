// SPDX-License-Identifier: MIT
pragma solidity ^0.8.20;

/// @title IPheromoneRegistry
/// @notice Interface for the PheromoneRegistry -- stigmergic coordination
///         through digital pheromones with exponential decay, alpha paradox,
///         graduated economics, and SINR interference modeling.
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
        uint64 blockNumber
    );

    /// @notice Emitted when a pheromone is confirmed by another agent.
    event PheromoneConfirmed(
        bytes32 indexed pheromoneId,
        address indexed confirmer,
        uint64 newConfirmationCount,
        uint64 newEffectiveHalfLife
    );

    /// @notice Emitted when a dead pheromone is cleaned up.
    event PheromoneDecayed(bytes32 indexed pheromoneId);

    /// @notice Emitted when deposit funds are returned to the depositor.
    event DepositReturned(
        bytes32 indexed pheromoneId,
        address indexed depositor,
        uint256 amount
    );

    /// @notice Emitted when a user withdraws their pending balance.
    event WithdrawalClaimed(address indexed recipient, uint256 amount);

    // ---------------------------------------------------------------
    // Write functions
    // ---------------------------------------------------------------

    /// @notice Deposit a pheromone at a location in HDC space.
    /// @param location 1,280-byte HDC vector identifying the region.
    /// @param pType The pheromone type (THREAT, OPPORTUNITY, or WISDOM).
    /// @param intensity Initial signal strength (100..10000 bps).
    function deposit(
        bytes calldata location,
        PheromoneType pType,
        uint64 intensity
    ) external payable;

    /// @notice Confirm an existing pheromone. Reduces its half-life
    ///         per the alpha paradox: new_hl = base_hl / (1 + n_confs).
    ///         First confirmer gets 10% of deposit; subsequent get 10%/n.
    /// @param pheromoneId The pheromone to confirm.
    function confirm(bytes32 pheromoneId) external;

    /// @notice Remove a dead pheromone (intensity < 1 bps).
    ///         Caller gets 5% of remaining deposit; rest returned to depositor.
    /// @param pheromoneId The dead pheromone to remove.
    function cleanup(bytes32 pheromoneId) external;

    /// @notice Batch-cleanup multiple dead pheromones in one transaction.
    /// @param pheromoneIds Array of pheromone IDs to clean up.
    function cleanupBatch(bytes32[] calldata pheromoneIds) external;

    /// @notice Withdraw accumulated pending balance (pull pattern).
    function withdraw() external;

    // ---------------------------------------------------------------
    // View functions
    // ---------------------------------------------------------------

    /// @notice Compute the current decayed intensity of a pheromone.
    /// @param pheromoneId The pheromone to query.
    /// @return intensity Current intensity in bps (0 = dead).
    function currentIntensity(bytes32 pheromoneId) external view returns (uint64 intensity);

    /// @notice Compute the SINR of a target pheromone against all
    ///         interferers of the same type at the same location.
    ///         SINR = signal / (noise + 1).
    /// @param pheromoneId The target pheromone.
    /// @return sinrValue SINR value.
    function sinr(bytes32 pheromoneId) external view returns (uint64 sinrValue);

    /// @notice Read pheromones at a location for a given type.
    /// @param locationHash Hash of the location to query.
    /// @param pType Filter by pheromone type.
    /// @return intensity Strongest current decayed intensity at location.
    /// @return sinrValue SINR of the strongest signal.
    function read(
        bytes32 locationHash,
        PheromoneType pType
    ) external view returns (uint64 intensity, uint64 sinrValue);

    /// @notice Read pheromones near a query location.
    /// @param queryVector 1,280-byte HDC vector.
    /// @param pType Filter by pheromone type.
    /// @param topK Number of results.
    /// @return ids Pheromone IDs.
    /// @return sinrValues SINR values.
    function readPheromones(
        bytes calldata queryVector,
        PheromoneType pType,
        uint8 topK
    ) external view returns (bytes32[] memory ids, uint64[] memory sinrValues);
}
