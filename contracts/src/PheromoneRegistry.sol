// SPDX-License-Identifier: MIT
pragma solidity ^0.8.20;

import {IPheromoneRegistry} from "./IPheromoneRegistry.sol";

/// @title PheromoneRegistry
/// @notice Stigmergic coordination via digital pheromones with exponential
///         decay (fixed-point integer math), alpha paradox, graduated
///         deposit economics, and SINR interference.
/// @dev All on-chain arithmetic is integer. No floating point.
///      Decay is computed at READ TIME -- no storage updates needed for decay.
contract PheromoneRegistry is IPheromoneRegistry {
    // ---------------------------------------------------------------
    // Constants
    // ---------------------------------------------------------------

    /// @dev Minimum deposit per pheromone.
    uint256 public constant MIN_DEPOSIT = 0.001 ether;

    /// @dev Minimum initial intensity (bps).
    uint64 public constant MIN_INTENSITY = 100;

    /// @dev Maximum initial intensity (bps).
    uint64 public constant MAX_INTENSITY = 10_000;

    /// @dev Death threshold: intensity below this is considered dead (1 bps).
    uint64 public constant DEATH_THRESHOLD = 1;

    // ---------------------------------------------------------------
    // Structs
    // ---------------------------------------------------------------

    struct Pheromone {
        bytes32 locationHash;
        address depositor;
        uint64  intensity;        // initial intensity in bps
        uint64  depositBlock;
        uint16  confirmationCount;
        uint8   pType;
    }

    // ---------------------------------------------------------------
    // Storage
    // ---------------------------------------------------------------

    /// @notice All pheromone deposits by unique ID.
    mapping(bytes32 => Pheromone) public pheromones;

    /// @notice Maps locationHash -> array of pheromoneIds at that location.
    mapping(bytes32 => bytes32[]) internal _locationPheromones;

    /// @notice Tracks which addresses have confirmed each pheromone.
    mapping(bytes32 => mapping(address => bool)) internal _confirmers;

    /// @notice Deposit amount tracked per pheromone.
    mapping(bytes32 => uint256) public deposits;

    /// @notice Remaining deposit balance per pheromone (after payouts).
    mapping(bytes32 => uint256) internal _remainingDeposit;

    /// @notice Pending withdrawals per address (pull pattern).
    mapping(address => uint256) public pendingWithdrawals;

    // ---------------------------------------------------------------
    // Half-life lookup
    // ---------------------------------------------------------------

    /// @dev Base half-lives in blocks per pheromone type.
    function _baseHalfLife(PheromoneType pType) internal pure returns (uint64) {
        if (pType == PheromoneType.THREAT)      return 100;
        if (pType == PheromoneType.OPPORTUNITY) return 250;
        if (pType == PheromoneType.WISDOM)      return 1000;
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
            msg.value >= MIN_DEPOSIT,
            "PheromoneRegistry: insufficient deposit"
        );

        bytes32 locationHash = keccak256(location);
        bytes32 pheromoneId = keccak256(
            abi.encodePacked(locationHash, msg.sender, block.number, uint8(pType))
        );

        require(
            pheromones[pheromoneId].depositBlock == 0,
            "PheromoneRegistry: ID collision"
        );

        pheromones[pheromoneId] = Pheromone({
            locationHash: locationHash,
            depositor: msg.sender,
            intensity: intensity,
            depositBlock: uint64(block.number),
            confirmationCount: 0,
            pType: uint8(pType)
        });

        deposits[pheromoneId] = msg.value;
        _remainingDeposit[pheromoneId] = msg.value;

        _locationPheromones[locationHash].push(pheromoneId);

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
    ///      First confirmer gets 10% of deposit.
    ///      Subsequent confirmers get 10% / confirmation_count.
    function confirm(bytes32 pheromoneId) external {
        Pheromone storage p = pheromones[pheromoneId];
        require(p.depositBlock != 0, "PheromoneRegistry: does not exist");
        require(
            !_confirmers[pheromoneId][msg.sender],
            "PheromoneRegistry: already confirmed"
        );
        require(
            _computeIntensity(p) >= DEATH_THRESHOLD,
            "PheromoneRegistry: pheromone is dead"
        );

        _confirmers[pheromoneId][msg.sender] = true;

        // Snapshot current intensity BEFORE changing half-life
        uint64 currentInt = _computeIntensity(p);

        p.confirmationCount++;

        // Reset depositBlock so decay restarts from now with shorter half-life
        p.intensity = currentInt;
        p.depositBlock = uint64(block.number);

        // Graduated confirmer reward
        uint256 remaining = _remainingDeposit[pheromoneId];
        uint256 reward;
        if (p.confirmationCount == 1) {
            // First confirmer: 10% of original deposit
            reward = deposits[pheromoneId] / 10;
        } else {
            // Subsequent: 10% / confirmation_count of original deposit
            reward = deposits[pheromoneId] / (10 * uint256(p.confirmationCount));
        }

        if (reward > remaining) {
            reward = remaining;
        }

        if (reward > 0) {
            _remainingDeposit[pheromoneId] = remaining - reward;
            pendingWithdrawals[msg.sender] += reward;
        }

        uint64 newHL = _effectiveHalfLife(p);

        emit PheromoneConfirmed(
            pheromoneId,
            msg.sender,
            p.confirmationCount,
            newHL
        );
    }

    /// @inheritdoc IPheromoneRegistry
    /// @dev Cleanup caller gets 5% of remaining deposit.
    ///      Rest returned to original depositor.
    function cleanup(bytes32 pheromoneId) external {
        Pheromone storage p = pheromones[pheromoneId];
        require(p.depositBlock != 0, "PheromoneRegistry: does not exist");
        require(
            _computeIntensity(p) < DEATH_THRESHOLD,
            "PheromoneRegistry: not dead yet"
        );

        bytes32 locHash = p.locationHash;
        address depositor = p.depositor;
        uint256 remaining = _remainingDeposit[pheromoneId];

        // Cleanup caller reward: 5% of remaining deposit
        uint256 cleanupReward = remaining / 20;
        uint256 depositorReturn = remaining - cleanupReward;

        if (cleanupReward > 0) {
            pendingWithdrawals[msg.sender] += cleanupReward;
        }
        if (depositorReturn > 0) {
            pendingWithdrawals[depositor] += depositorReturn;
            emit DepositReturned(pheromoneId, depositor, depositorReturn);
        }

        // Clear deposit tracking
        delete deposits[pheromoneId];
        delete _remainingDeposit[pheromoneId];

        // Remove from location index
        _removeFromLocationIndex(locHash, pheromoneId);

        // Clear pheromone storage
        delete pheromones[pheromoneId];

        emit PheromoneDecayed(pheromoneId);
    }

    /// @inheritdoc IPheromoneRegistry
    function cleanupBatch(bytes32[] calldata pheromoneIds) external {
        for (uint256 i = 0; i < pheromoneIds.length; i++) {
            bytes32 pid = pheromoneIds[i];
            Pheromone storage p = pheromones[pid];
            if (p.depositBlock == 0) continue;
            if (_computeIntensity(p) >= DEATH_THRESHOLD) continue;

            bytes32 locHash = p.locationHash;
            address depositor = p.depositor;
            uint256 remaining = _remainingDeposit[pid];

            uint256 cleanupReward = remaining / 20;
            uint256 depositorReturn = remaining - cleanupReward;

            if (cleanupReward > 0) {
                pendingWithdrawals[msg.sender] += cleanupReward;
            }
            if (depositorReturn > 0) {
                pendingWithdrawals[depositor] += depositorReturn;
                emit DepositReturned(pid, depositor, depositorReturn);
            }

            delete deposits[pid];
            delete _remainingDeposit[pid];

            _removeFromLocationIndex(locHash, pid);
            delete pheromones[pid];

            emit PheromoneDecayed(pid);
        }
    }

    /// @inheritdoc IPheromoneRegistry
    function withdraw() external {
        uint256 amount = pendingWithdrawals[msg.sender];
        require(amount > 0, "PheromoneRegistry: nothing to withdraw");

        pendingWithdrawals[msg.sender] = 0;

        emit WithdrawalClaimed(msg.sender, amount);

        (bool success,) = msg.sender.call{value: amount}("");
        require(success, "PheromoneRegistry: transfer failed");
    }

    // ---------------------------------------------------------------
    // View Functions
    // ---------------------------------------------------------------

    /// @inheritdoc IPheromoneRegistry
    function currentIntensity(bytes32 pheromoneId) external view returns (uint64) {
        Pheromone storage p = pheromones[pheromoneId];
        if (p.depositBlock == 0) return 0;
        return _computeIntensity(p);
    }

    /// @inheritdoc IPheromoneRegistry
    /// @dev SINR = signal / (noise + 1)
    function sinr(bytes32 pheromoneId) external view returns (uint64) {
        Pheromone storage target = pheromones[pheromoneId];
        if (target.depositBlock == 0) return 0;

        uint64 targetIntensity = _computeIntensity(target);
        if (targetIntensity < DEATH_THRESHOLD) return 0;

        uint64 noise = _computeNoise(pheromoneId, target.locationHash, target.pType);

        // SINR = signal / (noise + 1)
        return uint64(uint256(targetIntensity) / (uint256(noise) + 1));
    }

    /// @inheritdoc IPheromoneRegistry
    function read(
        bytes32 locationHash,
        PheromoneType pType
    ) external view returns (uint64 intensity, uint64 sinrValue) {
        bytes32[] storage atLocation = _locationPheromones[locationHash];

        bytes32 strongestId;
        uint64 strongestIntensity = 0;
        uint64 totalNoise = 0;

        // Find strongest pheromone of this type and sum all intensities
        for (uint256 i = 0; i < atLocation.length; i++) {
            bytes32 pid = atLocation[i];
            Pheromone storage p = pheromones[pid];
            if (p.depositBlock == 0) continue;
            if (p.pType != uint8(pType)) continue;

            uint64 pInt = _computeIntensity(p);
            if (pInt < DEATH_THRESHOLD) continue;

            if (pInt > strongestIntensity) {
                // Add previous strongest to noise
                totalNoise += strongestIntensity;
                strongestIntensity = pInt;
                strongestId = pid;
            } else {
                totalNoise += pInt;
            }
        }

        if (strongestIntensity == 0) return (0, 0);

        intensity = strongestIntensity;
        sinrValue = uint64(uint256(strongestIntensity) / (uint256(totalNoise) + 1));
    }

    /// @inheritdoc IPheromoneRegistry
    function readPheromones(
        bytes calldata queryVector,
        PheromoneType pType,
        uint8 topK
    ) external view returns (bytes32[] memory ids, uint64[] memory sinrValues) {
        // Placeholder: full implementation requires precompile search
        ids = new bytes32[](0);
        sinrValues = new uint64[](0);
    }

    // ---------------------------------------------------------------
    // Internal: Fixed-Point Decay
    // ---------------------------------------------------------------

    /// @dev Compute the effective half-life after alpha paradox reduction.
    ///      effective_half_life = base_half_life / (1 + confirmation_count)
    function _effectiveHalfLife(
        Pheromone storage p
    ) internal view returns (uint64) {
        uint64 base = _baseHalfLife(PheromoneType(p.pType));
        return base / (1 + uint64(p.confirmationCount));
    }

    /// @dev Compute decayed intensity using fixed-point integer arithmetic.
    ///      Formula: intensity_0 * 2^(-(age) / half_life)
    ///
    ///      Decomposition:
    ///        Let q = age / half_life  (number of full halvings)
    ///        Let r = age % half_life  (remainder)
    ///        Then 2^(-age/hl) = 2^(-q) * 2^(-r/hl)
    ///
    ///      2^(-q) is implemented as right-shift by q.
    ///      2^(-r/hl) is approximated via linear interpolation:
    ///        2^(-r/hl) ~ (2*hl - r) / (2*hl)
    ///      Maps r=0 -> 1.0, r=hl -> 0.5 (exact endpoints).
    function _computeIntensity(
        Pheromone storage p
    ) internal view returns (uint64) {
        uint64 age = uint64(block.number) - p.depositBlock;
        uint64 hl = _effectiveHalfLife(p);

        if (hl == 0) return 0;

        uint64 q = age / hl;  // number of full halvings
        uint64 r = age % hl;  // remainder

        // After 20+ halvings, intensity is effectively 0
        if (q >= 20) return 0;

        // Integer part: right-shift by q halvings
        uint64 integerDecay = p.intensity >> q;
        if (integerDecay == 0) return 0;

        // Fractional part: linear interpolation of 2^(-r/hl)
        uint64 numerator = 2 * hl - r;
        uint64 denominator = 2 * hl;

        // Use uint128 for intermediate to prevent overflow
        uint128 result = (uint128(integerDecay) * uint128(numerator)) / uint128(denominator);

        if (result > type(uint64).max) return type(uint64).max;
        return uint64(result);
    }

    /// @dev Sum noise (all interferer intensities) for a pheromone at a location.
    function _computeNoise(
        bytes32 pheromoneId,
        bytes32 locationHash,
        uint8 pType
    ) internal view returns (uint64) {
        bytes32[] storage atLocation = _locationPheromones[locationHash];
        uint64 noise = 0;

        for (uint256 i = 0; i < atLocation.length; i++) {
            bytes32 otherId = atLocation[i];
            if (otherId == pheromoneId) continue;

            Pheromone storage other = pheromones[otherId];
            if (other.depositBlock == 0) continue;
            if (other.pType != pType) continue;

            uint64 otherInt = _computeIntensity(other);
            if (otherInt >= DEATH_THRESHOLD) {
                noise += otherInt;
            }
        }

        return noise;
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
