# PheromoneRegistry.sol -- Stigmergic Signal Layer

> **Status: DIVERGED** -- PR #42 moved stigmergy from Solidity to a Rust
> precompile (`StigmergyPrecompile` at `0xA0D`). This document describes the
> original Solidity design. The actual implementation lives in Rust. See the
> [Reconciliation](#reconciliation-solidity-vs-rust-precompile) section below.

## Goal

Implement the `PheromoneRegistry` Solidity smart contract. This contract provides
a stigmergic coordination layer: agents deposit, confirm, read, and clean up
short-lived signals (pheromones) on-chain. The pheromone system is inspired by
biological ant colony optimization (Dorigo, 1996) and adapted for a blockchain
context where the "environment" is the global state of a distributed ledger.

The contract is self-contained. It does NOT depend on the InsightBoard, the
ReputationRegistry, or the HDC precompile. It can be compiled and tested in
isolation.

**Source design document:** `07-shared-substrate.md`, sections "Pheromone Types"
through "Cleanup Mechanism".

---

## Background: Key Concepts

Before writing code, understand three things:

### 1. Stigmergy

Agents never talk to each other. They read and write a shared environment (the
blockchain). A pheromone deposit modifies the environment. Other agents observe
the modification and change their behavior. Coordination emerges without direct
communication.

### 2. The Alpha Paradox

When a pheromone is confirmed (another agent independently validates the signal),
its half-life is **reduced**, not extended. This is counter-intuitive: confirmation
is "good," so why does the signal decay faster?

Because each confirmation means the information is spreading. By the time 10
agents know about a threat, that threat is common knowledge. The signal's value
*as a signal* has decreased -- it carries no new information for remaining agents.
This mirrors the Efficient Market Hypothesis (Fama, 1970): once information is
widely known, it is priced in.

The formula:

```
effective_half_life = base_half_life / (1 + confirmation_count)
```

Practical effects:
- **Scouts are rewarded.** The first depositor gets the signal at full half-life.
  Later arrivals find a shorter half-life (less remaining value).
- **Echo chambers are prevented.** Popular signals decay faster, creating space
  for new information.
- **Sybil amplification is defeated.** An attacker who confirms a false signal
  with Sybil accounts *accelerates its decay*, achieving the opposite of what
  they intended.

Worked example (THREAT, base half-life = 100 blocks):

| Confirmations | Effective Half-Life | Time to Death (~10 half-lives) |
|---------------|--------------------|-----------------------------|
| 0             | 100 blocks         | ~1,000 blocks (~400s)       |
| 1             | 50 blocks          | ~500 blocks (~200s)         |
| 2             | 33 blocks          | ~330 blocks (~132s)         |
| 5             | 17 blocks          | ~170 blocks (~68s)          |
| 10            | 9 blocks           | ~90 blocks (~36s)           |

### 3. Fixed-Point Decay (Consensus Safety)

**NEVER use floating-point arithmetic in any function that executes on-chain.**
The EVM does not have native float types, and even if you emulated them,
`f64::powf()` is a transcendental function whose result varies by platform,
compiler, and optimization level. Two validators computing the same decay with
floats can get different results, breaking consensus.

All decay math uses integer arithmetic only.

---

## Contract Specification

### Data Types

```solidity
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.20;

contract PheromoneRegistry {
    // ---------------------------------------------------------------
    // Types
    // ---------------------------------------------------------------

    /// @dev Pheromone types ordered by increasing half-life.
    ///      THREAT = 0, OPPORTUNITY = 1, WISDOM = 2.
    enum PheromoneType { THREAT, OPPORTUNITY, WISDOM }

    /// @dev On-chain record for a single pheromone deposit.
    ///      Packed into 2 storage slots (64 bytes):
    ///        Slot 1: vectorHash       (32 bytes)
    ///        Slot 2: depositor (20) + depositBlock (8) +
    ///                initialIntensity (2) + pheromoneType (1) +
    ///                confirmations (4) = 35 bytes --> packed into 32
    ///                (Solidity packs trailing fields into the same slot
    ///                 when they fit; 20+8+2+1+4 = 35 > 32, so this
    ///                 actually requires 3 slots. See layout note below.)
    ///
    ///      ACTUAL LAYOUT (3 storage slots):
    ///        Slot 0: vectorHash       (bytes32, 32 bytes)
    ///        Slot 1: depositor        (address, 20 bytes) --|
    ///                depositBlock     (uint64,   8 bytes) --| = 28 bytes
    ///        Slot 2: initialIntensity (uint16,   2 bytes) --|
    ///                pheromoneType    (uint8,    1 byte)  --|
    ///                confirmations    (uint32,   4 bytes) --| = 7 bytes
    struct PheromoneDeposit {
        bytes32 vectorHash;        // keccak256 of the HDC vector
        address depositor;         // who deposited
        uint64  depositBlock;      // block.number at deposit time
        uint16  initialIntensity;  // e.g. 1000 (max 10000)
        uint8   pheromoneType;     // 0=THREAT, 1=OPPORTUNITY, 2=WISDOM
        uint32  confirmations;     // number of independent confirmations
    }
```

### Constants

```solidity
    // ---------------------------------------------------------------
    // Constants
    // ---------------------------------------------------------------

    /// @dev Base half-lives in blocks, indexed by PheromoneType.
    ///      THREAT=100, OPPORTUNITY=250, WISDOM=1000.
    uint64[3] internal HALF_LIVES = [uint64(100), uint64(250), uint64(1000)];

    /// @dev Pheromone is considered dead when intensity falls below this.
    ///      In the same units as initialIntensity. At death threshold 1,
    ///      a standard 1000-intensity pheromone needs ~10 half-lives to die.
    uint256 constant DEATH_THRESHOLD = 1;

    /// @dev Noise floor for SINR computation. Prevents division by zero
    ///      and models background uncertainty. Units: same as intensity.
    uint256 constant NOISE_FLOOR = 10;

    /// @dev Precision multiplier for fixed-point decay math.
    ///      All intermediate intensity values are scaled by this factor.
    ///      Final results are divided by PRECISION to get actual intensity.
    uint256 constant PRECISION = 1e18;

    /// @dev Scale factor for SINR return values (basis points x 100).
    ///      SINR of 1.0 = 1_000_000. SINR of 0.01 = 10_000.
    uint256 constant SINR_SCALE = 1_000_000;

    /// @dev Minimum allowed intensity on deposit.
    uint16 constant MIN_INTENSITY = 100;

    /// @dev Maximum allowed intensity on deposit.
    uint16 constant MAX_INTENSITY = 10000;
```

### State Variables

```solidity
    // ---------------------------------------------------------------
    // Storage
    // ---------------------------------------------------------------

    /// @dev Auto-incrementing ID for pheromone deposits.
    uint256 public nextId;

    /// @dev All pheromone deposits, keyed by ID.
    mapping(uint256 => PheromoneDeposit) public pheromones;

    /// @dev Track who has confirmed which pheromone (prevent double-confirm).
    mapping(uint256 => mapping(address => bool)) public hasConfirmed;
```

### Events

```solidity
    // ---------------------------------------------------------------
    // Events
    // ---------------------------------------------------------------

    event PheromoneDeposited(
        uint256 indexed id,
        address indexed depositor,
        uint8   pheromoneType,
        uint16  intensity
    );

    event PheromoneConfirmed(
        uint256 indexed id,
        uint32  confirmations,
        uint64  newHalfLife
    );

    event PheromoneCleaned(uint256 indexed id);
```

### Function: `deposit`

Creates a new pheromone. Gas: ~80,000.

```solidity
    // ---------------------------------------------------------------
    // deposit
    // ---------------------------------------------------------------

    /// @notice Deposit a pheromone signal.
    /// @param vectorHash  keccak256 of the HDC vector identifying the location.
    /// @param pheromoneType  0=THREAT, 1=OPPORTUNITY, 2=WISDOM.
    /// @param intensity  Initial signal strength (100-10000).
    /// @return id  The ID of the newly created pheromone.
    function deposit(
        bytes32 vectorHash,
        uint8   pheromoneType,
        uint16  intensity
    ) external returns (uint256 id) {
        require(pheromoneType <= 2, "Invalid pheromone type");
        require(intensity >= MIN_INTENSITY, "Intensity too low");
        require(intensity <= MAX_INTENSITY, "Intensity too high");

        id = nextId++;

        pheromones[id] = PheromoneDeposit({
            vectorHash:       vectorHash,
            depositor:        msg.sender,
            depositBlock:     uint64(block.number),
            initialIntensity: intensity,
            pheromoneType:    pheromoneType,
            confirmations:    0
        });

        emit PheromoneDeposited(id, msg.sender, pheromoneType, intensity);
    }
```

**Gas breakdown:**
- 2 cold SSTORE (new slots, zero-to-nonzero): 2 x 22,100 = 44,200
- 1 warm SSTORE (third slot): 22,100
- Event emission: ~1,500 (375 base + 750 topics + ~375 data)
- Calldata + checks + counter increment: ~12,000
- **Total: ~80,000 gas**

### Function: `confirm`

Confirm a signal. Implements the alpha paradox. Gas: ~30,000.

```solidity
    // ---------------------------------------------------------------
    // confirm
    // ---------------------------------------------------------------

    /// @notice Confirm a pheromone signal. Each confirmation REDUCES the
    ///         effective half-life (alpha paradox): widely-known signals
    ///         decay faster because they carry less marginal information.
    /// @param pheromoneId  The ID of the pheromone to confirm.
    function confirm(uint256 pheromoneId) external {
        PheromoneDeposit storage p = pheromones[pheromoneId];
        require(p.depositBlock != 0, "Pheromone does not exist");
        require(p.depositor != msg.sender, "Cannot confirm own pheromone");
        require(!hasConfirmed[pheromoneId][msg.sender], "Already confirmed");

        // Must still be alive.
        require(
            currentIntensity(pheromoneId) >= DEATH_THRESHOLD,
            "Pheromone is dead"
        );

        hasConfirmed[pheromoneId][msg.sender] = true;
        p.confirmations += 1;

        // Compute new effective half-life for the event.
        uint64 baseHL = HALF_LIVES[p.pheromoneType];
        uint64 newHL = baseHL / (1 + uint64(p.confirmations));

        emit PheromoneConfirmed(pheromoneId, p.confirmations, newHL);
    }
```

**Gas breakdown:**
- 1 cold SLOAD (read deposit): 2,100
- 1 cold SLOAD (read hasConfirmed): 2,100
- 1 cold SSTORE (write hasConfirmed, zero-to-nonzero): 22,100
- 1 warm SSTORE (update confirmations): 2,900
- currentIntensity view call: ~1,000 (compute only, no storage writes)
- Event emission: ~1,500
- **Total: ~30,000 gas**

### Function: `currentIntensity`

Compute the decayed intensity at the current block. This is a `view` function --
it reads storage but writes nothing. Gas: free when called externally via
`eth_call`; ~3,000 when called internally.

```solidity
    // ---------------------------------------------------------------
    // currentIntensity -- fixed-point exponential decay
    // ---------------------------------------------------------------

    /// @notice Compute the current intensity of a pheromone after decay.
    /// @dev    Uses pure integer arithmetic. NO floating point.
    ///
    ///         Formula: intensity_0 * 2^(-elapsed / effective_half_life)
    ///
    ///         Decomposed as:
    ///           Let q = elapsed / effective_half_life   (integer division)
    ///           Let r = elapsed % effective_half_life   (remainder)
    ///           Then: result = intensity_0 >> q          (integer part)
    ///                        * fractionalDecay(r, hl)    (fractional part)
    ///                        / PRECISION
    ///
    ///         The fractional part 2^(-r/hl) is in [0.5, 1.0) and is
    ///         approximated via linear interpolation in fixed-point.
    ///
    /// @param pheromoneId  The ID of the pheromone.
    /// @return intensity   The current intensity (same units as initialIntensity).
    function currentIntensity(uint256 pheromoneId) public view returns (uint256) {
        PheromoneDeposit storage p = pheromones[pheromoneId];
        if (p.depositBlock == 0) return 0;

        uint256 elapsed = block.number - uint256(p.depositBlock);
        if (elapsed == 0) return uint256(p.initialIntensity);

        // Effective half-life: alpha paradox.
        uint256 baseHL = uint256(HALF_LIVES[p.pheromoneType]);
        uint256 effectiveHL = baseHL / (1 + uint256(p.confirmations));
        if (effectiveHL == 0) return 0; // Saturated confirmations.

        // Decompose: q = integer half-lives, r = remainder blocks.
        uint256 q = elapsed / effectiveHL;
        uint256 r = elapsed % effectiveHL;

        // If q >= 64, intensity has been right-shifted to zero.
        // (initialIntensity is at most 10000 < 2^14, so q >= 14 already
        // guarantees 0. We use 64 as a safe upper bound.)
        if (q >= 64) return 0;

        // Integer part: right-shift by q (divide by 2^q).
        uint256 intPart = uint256(p.initialIntensity) >> q;
        if (intPart == 0) return 0;

        // Fractional part: 2^(-r/effectiveHL) for r in [0, effectiveHL).
        // This value is in [0.5, 1.0).
        //
        // We use linear interpolation:
        //   2^(-r/hl) ~ 1 - r * (1 - 0.5) / hl = 1 - r / (2 * hl)
        //
        // In fixed-point (scaled by PRECISION):
        //   fracDecay = PRECISION - (r * PRECISION) / (2 * effectiveHL)
        //
        // This linear approximation has maximum error at r = effectiveHL:
        //   true value = 0.5, approximation = 0.5 -> exact at endpoints.
        //   Maximum error is at r ~ 0.33*hl: true ~ 0.794, approx ~ 0.833,
        //   error ~ 5%. Acceptable for pheromone decay.
        //
        // For higher precision, use the piecewise method below instead.
        uint256 fracDecay = PRECISION - (r * PRECISION) / (2 * effectiveHL);

        return (intPart * fracDecay) / PRECISION;
    }
```

#### Higher-Precision Alternative: Piecewise Lookup

If the 5% error of linear interpolation is unacceptable, replace the fractional
part with a lookup table. The idea: precompute `2^(-k/N)` for `k = 0..N-1`
where N is the number of steps within one half-life.

```solidity
    /// @dev Lookup table for 2^(-k/8) * PRECISION, k = 0..7.
    ///      These are the fractional decay multipliers for 8 sub-intervals
    ///      within a single half-life.
    ///
    ///      2^(-0/8) = 1.0000  ->  1000000000000000000
    ///      2^(-1/8) = 0.9170  ->   917004043204671230
    ///      2^(-2/8) = 0.8409  ->   840896415253714543
    ///      2^(-3/8) = 0.7711  ->   771105412703970372
    ///      2^(-4/8) = 0.7071  ->   707106781186547524
    ///      2^(-5/8) = 0.6484  ->   648419777325504237
    ///      2^(-6/8) = 0.5946  ->   594603557501360533
    ///      2^(-7/8) = 0.5453  ->   545253888261690457
    uint256[8] internal FRAC_TABLE = [
        1000000000000000000,
        917004043204671230,
        840896415253714543,
        771105412703970372,
        707106781186547524,
        648419777325504237,
        594603557501360533,
        545253888261690457
    ];

    /// @dev Lookup-based fractional decay. Divides the remainder r into
    ///      8 buckets within the half-life and uses precomputed multipliers.
    ///      Maximum error: ~0.5% (vs. 5% for linear).
    function _fracDecayLookup(uint256 r, uint256 hl)
        internal view returns (uint256)
    {
        // Which bucket does r fall into?  k = r * 8 / hl
        uint256 k = (r * 8) / hl;
        if (k >= 8) k = 7; // Safety clamp (should not happen).
        return FRAC_TABLE[k];
    }
```

To use the lookup version, replace the `fracDecay` computation in
`currentIntensity`:

```solidity
        // Replace the linear interpolation block with:
        uint256 fracDecay = _fracDecayLookup(r, effectiveHL);
```

### Function: `sinr`

Compute the Signal-to-Interference-plus-Noise Ratio for a pheromone.

**Important:** A full on-chain SINR computation requires iterating over all
pheromones of the same type in the same region. This is expensive (O(n) storage
reads). The contract provides two options:

1. **Precomputed SINR** -- the caller passes in the interferer IDs (computed
   off-chain) and the contract verifies and computes. This is the recommended
   approach.

2. **Pure view helper** -- for testing or low-pheromone-count scenarios.

```solidity
    // ---------------------------------------------------------------
    // sinr -- Signal-to-Interference-plus-Noise Ratio
    // ---------------------------------------------------------------

    /// @notice Compute the SINR of a target pheromone against a set of
    ///         interfering pheromones.
    /// @dev    SINR = signal / (sum_of_interferers + noise_floor)
    ///         Result is scaled by SINR_SCALE (1,000,000 = SINR of 1.0).
    ///
    ///         The caller provides the interferer IDs. In production,
    ///         these are discovered off-chain (by HDC similarity search)
    ///         and passed in. The contract does NOT verify that the
    ///         interferers are actually in the same HDC region -- that
    ///         verification happens at the application layer.
    ///
    /// @param targetId      The pheromone whose signal quality we measure.
    /// @param interfererIds IDs of pheromones that interfere with the target.
    ///                      Must all have the same pheromoneType as the target.
    /// @return sinrValue    Scaled SINR (1,000,000 = 1.0).
    function sinr(
        uint256 targetId,
        uint256[] calldata interfererIds
    ) external view returns (uint256 sinrValue) {
        uint256 signal = currentIntensity(targetId);
        if (signal == 0) return 0;

        uint8 targetType = pheromones[targetId].pheromoneType;

        uint256 interference = 0;
        for (uint256 i = 0; i < interfererIds.length; i++) {
            uint256 iid = interfererIds[i];
            require(iid != targetId, "Target cannot be its own interferer");
            require(
                pheromones[iid].pheromoneType == targetType,
                "Interferer type mismatch"
            );
            interference += currentIntensity(iid);
        }

        // SINR = signal / (interference + noise_floor)
        // Scaled: sinrValue = signal * SINR_SCALE / (interference + NOISE_FLOOR)
        sinrValue = (signal * SINR_SCALE) / (interference + NOISE_FLOOR);
    }
```

#### SINR Worked Examples

All examples assume `initialIntensity = 1000`, same deposit block, zero elapsed
blocks (no decay yet), `NOISE_FLOOR = 10`, `SINR_SCALE = 1,000,000`.

**1 pheromone (no interferers):**
```
signal = 1000
interference = 0
SINR = 1000 * 1,000,000 / (0 + 10) = 100,000,000
     = 100.0 (after dividing by SINR_SCALE)
```

**2 identical pheromones (1 interferer):**
```
signal = 1000
interference = 1000
SINR = 1000 * 1,000,000 / (1000 + 10) = 990,099
     ~ 0.99
```

**10 identical pheromones (9 interferers):**
```
signal = 1000
interference = 9 * 1000 = 9000
SINR = 1000 * 1,000,000 / (9000 + 10) = 110,987
     ~ 0.11
```

**100 identical pheromones (99 interferers):**
```
signal = 1000
interference = 99 * 1000 = 99,000
SINR = 1000 * 1,000,000 / (99,000 + 10) = 10,100
     ~ 0.01
```

This demonstrates the anti-Sybil property: adding more pheromones *reduces* the
effective signal quality of each one. An attacker depositing 100 copies of the
same signal achieves SINR of 0.01, not 100x amplification.

### Function: `cleanup`

Remove dead pheromones and reclaim storage. Gas: net negative per cleaned slot
(storage refund via EIP-3529).

```solidity
    // ---------------------------------------------------------------
    // cleanup
    // ---------------------------------------------------------------

    /// @notice Remove dead pheromones (intensity < DEATH_THRESHOLD).
    ///         Anyone can call this. The caller receives storage refund gas.
    /// @param pheromoneIds  Array of pheromone IDs to attempt to clean.
    function cleanup(uint256[] calldata pheromoneIds) external {
        for (uint256 i = 0; i < pheromoneIds.length; i++) {
            uint256 id = pheromoneIds[i];
            PheromoneDeposit storage p = pheromones[id];

            // Skip non-existent entries silently (idempotent).
            if (p.depositBlock == 0) continue;

            // Only clean if actually dead.
            if (currentIntensity(id) >= DEATH_THRESHOLD) continue;

            // Clear storage (triggers SSTORE refund).
            delete pheromones[id];

            emit PheromoneCleaned(id);
        }
    }
```

**Gas breakdown per cleaned pheromone:**
- currentIntensity computation: ~1,000
- 3 SSTORE-to-zero (clear 3 slots): 3 x refund of 4,800 = -14,400 refund
- Event emission: ~1,000
- Net per pheromone: ~2,000 execution - 14,400 refund = **net ~-9,600 gas** (negative = caller profits)

Note: EIP-3529 caps the total refund at 20% of the transaction's gas used. In
practice, batch-cleaning many pheromones in one transaction maximizes the
refund ratio.

### Closing Brace

```solidity
}
```

---

## Complete Contract (Copy-Paste Ready)

```solidity
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.20;

/// @title PheromoneRegistry -- Stigmergic Signal Layer
/// @notice Agents deposit, confirm, read, and clean up short-lived
///         pheromone signals for stigmergic coordination.
///         Decay is computed at read time (lazy evaluation).
///         Confirmation accelerates decay (alpha paradox).
contract PheromoneRegistry {
    // ---------------------------------------------------------------
    // Types
    // ---------------------------------------------------------------

    enum PheromoneType { THREAT, OPPORTUNITY, WISDOM }

    struct PheromoneDeposit {
        bytes32 vectorHash;
        address depositor;
        uint64  depositBlock;
        uint16  initialIntensity;
        uint8   pheromoneType;
        uint32  confirmations;
    }

    // ---------------------------------------------------------------
    // Constants
    // ---------------------------------------------------------------

    uint64[3] internal HALF_LIVES = [uint64(100), uint64(250), uint64(1000)];

    uint256 constant DEATH_THRESHOLD = 1;
    uint256 constant NOISE_FLOOR     = 10;
    uint256 constant PRECISION       = 1e18;
    uint256 constant SINR_SCALE      = 1_000_000;
    uint16  constant MIN_INTENSITY   = 100;
    uint16  constant MAX_INTENSITY   = 10000;

    // ---------------------------------------------------------------
    // Fractional decay lookup table: 2^(-k/8) * PRECISION, k = 0..7
    // ---------------------------------------------------------------

    uint256[8] internal FRAC_TABLE = [
        1000000000000000000, // 2^(-0/8) = 1.0000
         917004043204671230, // 2^(-1/8) = 0.9170
         840896415253714543, // 2^(-2/8) = 0.8409
         771105412703970372, // 2^(-3/8) = 0.7711
         707106781186547524, // 2^(-4/8) = 0.7071
         648419777325504237, // 2^(-5/8) = 0.6484
         594603557501360533, // 2^(-6/8) = 0.5946
         545253888261690457  // 2^(-7/8) = 0.5453
    ];

    // ---------------------------------------------------------------
    // Storage
    // ---------------------------------------------------------------

    uint256 public nextId;
    mapping(uint256 => PheromoneDeposit) public pheromones;
    mapping(uint256 => mapping(address => bool)) public hasConfirmed;

    // ---------------------------------------------------------------
    // Events
    // ---------------------------------------------------------------

    event PheromoneDeposited(
        uint256 indexed id,
        address indexed depositor,
        uint8   pheromoneType,
        uint16  intensity
    );

    event PheromoneConfirmed(
        uint256 indexed id,
        uint32  confirmations,
        uint64  newHalfLife
    );

    event PheromoneCleaned(uint256 indexed id);

    // ---------------------------------------------------------------
    // deposit
    // ---------------------------------------------------------------

    function deposit(
        bytes32 vectorHash,
        uint8   pheromoneType,
        uint16  intensity
    ) external returns (uint256 id) {
        require(pheromoneType <= 2, "Invalid pheromone type");
        require(intensity >= MIN_INTENSITY, "Intensity too low");
        require(intensity <= MAX_INTENSITY, "Intensity too high");

        id = nextId++;

        pheromones[id] = PheromoneDeposit({
            vectorHash:       vectorHash,
            depositor:        msg.sender,
            depositBlock:     uint64(block.number),
            initialIntensity: intensity,
            pheromoneType:    pheromoneType,
            confirmations:    0
        });

        emit PheromoneDeposited(id, msg.sender, pheromoneType, intensity);
    }

    // ---------------------------------------------------------------
    // confirm
    // ---------------------------------------------------------------

    function confirm(uint256 pheromoneId) external {
        PheromoneDeposit storage p = pheromones[pheromoneId];
        require(p.depositBlock != 0, "Pheromone does not exist");
        require(p.depositor != msg.sender, "Cannot confirm own pheromone");
        require(!hasConfirmed[pheromoneId][msg.sender], "Already confirmed");
        require(
            currentIntensity(pheromoneId) >= DEATH_THRESHOLD,
            "Pheromone is dead"
        );

        hasConfirmed[pheromoneId][msg.sender] = true;
        p.confirmations += 1;

        uint64 baseHL = HALF_LIVES[p.pheromoneType];
        uint64 newHL = baseHL / (1 + uint64(p.confirmations));

        emit PheromoneConfirmed(pheromoneId, p.confirmations, newHL);
    }

    // ---------------------------------------------------------------
    // currentIntensity
    // ---------------------------------------------------------------

    function currentIntensity(uint256 pheromoneId)
        public view returns (uint256)
    {
        PheromoneDeposit storage p = pheromones[pheromoneId];
        if (p.depositBlock == 0) return 0;

        uint256 elapsed = block.number - uint256(p.depositBlock);
        if (elapsed == 0) return uint256(p.initialIntensity);

        uint256 baseHL = uint256(HALF_LIVES[p.pheromoneType]);
        uint256 effectiveHL = baseHL / (1 + uint256(p.confirmations));
        if (effectiveHL == 0) return 0;

        uint256 q = elapsed / effectiveHL;
        uint256 r = elapsed % effectiveHL;

        if (q >= 64) return 0;

        uint256 intPart = uint256(p.initialIntensity) >> q;
        if (intPart == 0) return 0;

        // Fractional decay via lookup table.
        uint256 k = (r * 8) / effectiveHL;
        if (k >= 8) k = 7;
        uint256 fracDecay = FRAC_TABLE[k];

        return (intPart * fracDecay) / PRECISION;
    }

    // ---------------------------------------------------------------
    // sinr
    // ---------------------------------------------------------------

    function sinr(
        uint256 targetId,
        uint256[] calldata interfererIds
    ) external view returns (uint256 sinrValue) {
        uint256 signal = currentIntensity(targetId);
        if (signal == 0) return 0;

        uint8 targetType = pheromones[targetId].pheromoneType;

        uint256 interference = 0;
        for (uint256 i = 0; i < interfererIds.length; i++) {
            uint256 iid = interfererIds[i];
            require(iid != targetId, "Target cannot be its own interferer");
            require(
                pheromones[iid].pheromoneType == targetType,
                "Interferer type mismatch"
            );
            interference += currentIntensity(iid);
        }

        sinrValue = (signal * SINR_SCALE) / (interference + NOISE_FLOOR);
    }

    // ---------------------------------------------------------------
    // cleanup
    // ---------------------------------------------------------------

    function cleanup(uint256[] calldata pheromoneIds) external {
        for (uint256 i = 0; i < pheromoneIds.length; i++) {
            uint256 id = pheromoneIds[i];
            PheromoneDeposit storage p = pheromones[id];
            if (p.depositBlock == 0) continue;
            if (currentIntensity(id) >= DEATH_THRESHOLD) continue;
            delete pheromones[id];
            emit PheromoneCleaned(id);
        }
    }
}
```

---

## Fixed-Point Decay Math: Detailed Walkthrough

### The Problem

We need to compute `intensity_0 * 2^(-elapsed / half_life)` using only integer
arithmetic. The EVM has no floating-point instructions. Even if you used an
off-chain Rust implementation, `f64::powf()` is non-deterministic across
platforms.

### The Decomposition

Any rational exponent `elapsed / half_life` can be split into integer and
fractional parts:

```
Let q = elapsed / half_life     (integer division, rounds down)
Let r = elapsed % half_life     (remainder)

Then:  2^(-elapsed/half_life) = 2^(-q) * 2^(-r/half_life)
```

- `2^(-q)` is trivial: right-shift by `q` bits. This is the "integer part" of
  the decay.
- `2^(-r/half_life)` is the "fractional part." Since `0 <= r < half_life`, this
  value is always in the range `[0.5, 1.0)`. We approximate it with a lookup
  table.

### Worked Example 1: Basic Decay

THREAT pheromone, `intensity_0 = 1000`, deposited at block 50,000.
No confirmations, so `effectiveHL = 100`.

**At block 50,150 (elapsed = 150):**

```
q = 150 / 100 = 1
r = 150 % 100 = 50

Integer part:  1000 >> 1 = 500
Fractional:    k = (50 * 8) / 100 = 4  ->  FRAC_TABLE[4] = 707106781186547524
               This represents 2^(-4/8) = 2^(-0.5) = 0.7071

Result:  (500 * 707106781186547524) / 1e18 = 353
```

True value: `1000 * 2^(-1.5) = 1000 * 0.3536 = 353.6`. Our integer result is
353. Error: 0.17%.

**At block 50,100 (elapsed = 100, exactly 1 half-life):**

```
q = 100 / 100 = 1
r = 100 % 100 = 0

Integer part:  1000 >> 1 = 500
Fractional:    k = (0 * 8) / 100 = 0  ->  FRAC_TABLE[0] = 1e18 (= 1.0)

Result:  (500 * 1e18) / 1e18 = 500
```

Exact. As expected -- at one half-life, intensity = 50%.

**At block 50,700 (elapsed = 700, 7 half-lives):**

```
q = 700 / 100 = 7
r = 700 % 100 = 0

Integer part:  1000 >> 7 = 7   (1000 / 128 = 7.8125, truncated)
Fractional:    k = 0  ->  1.0

Result:  7
```

True value: `1000 * 2^(-7) = 7.8125`. Our result is 7. The truncation from
right-shift is acceptable; both values are near the death threshold.

### Worked Example 2: Alpha Paradox Decay

OPPORTUNITY pheromone, `intensity_0 = 1000`, deposited at block 80,000.
After 3 confirmations: `effectiveHL = 250 / (1+3) = 62`.

**At block 80,062 (elapsed = 62, exactly 1 effective half-life):**

```
q = 62 / 62 = 1
r = 62 % 62 = 0

Integer part:  1000 >> 1 = 500
Fractional:    1.0

Result:  500
```

Without confirmations, 62 blocks would give:
```
q = 62 / 250 = 0
r = 62 % 250 = 62
k = (62 * 8) / 250 = 1  ->  FRAC_TABLE[1] = 917004043204671230

Result: (1000 * 917004043204671230) / 1e18 = 917
```

Same pheromone, same elapsed time: 500 (confirmed) vs. 917 (unconfirmed). The
3 confirmations caused it to lose 45% more intensity. That is the alpha paradox
in action.

### Overflow Safety

The largest intermediate value in `currentIntensity` is:

```
intPart * fracDecay = MAX_INTENSITY * PRECISION = 10000 * 1e18 = 1e22
```

`1e22` fits in a `uint256` (max ~1.16e77). No overflow risk.

---

## Gas Cost Summary

| Operation | Gas | Notes |
|-----------|-----|-------|
| `deposit` | ~80,000 | 2-3 cold SSTORE + event |
| `confirm` | ~30,000 | 1 cold SSTORE (hasConfirmed) + 1 warm SSTORE (confirmations) + event |
| `currentIntensity` (external call) | 0 | View function, free via eth_call |
| `currentIntensity` (internal call) | ~3,000 | Computation only, no storage writes |
| `sinr` (external, N interferers) | ~3,000 * N | View function, N reads of currentIntensity |
| `cleanup` (per pheromone) | net ~-9,600 | SSTORE-to-zero refunds minus computation |
| `cleanup` (batch of 10) | net ~-75,000 | Amortized transaction base cost |

---

## Anti-Patterns

These are mistakes that will break the contract. Do not make them.

### 1. DO NOT use floating-point arithmetic

```solidity
// WRONG -- consensus violation. f64 results differ across validators.
// (Solidity does not even support float, but do not try to emulate it
// with assembly or external calls to a Rust precompile that uses f64.)
function badDecay(uint256 elapsed, uint256 hl) pure returns (uint256) {
    // This is pseudocode -- Solidity can't do this, but the principle
    // applies to any off-chain computation used for consensus:
    return uint256(1000.0 * pow(2.0, -float(elapsed) / float(hl)));
}
```

Use integer decomposition (bit shift + lookup table) as shown above.

### 2. DO NOT forget the alpha paradox

```solidity
// WRONG -- using base half-life without accounting for confirmations.
function badIntensity(uint256 id) view returns (uint256) {
    uint256 elapsed = block.number - pheromones[id].depositBlock;
    uint256 hl = HALF_LIVES[pheromones[id].pheromoneType];
    // BUG: ignores confirmations. A pheromone with 10 confirmations
    // would decay at the same rate as one with 0. This defeats the
    // alpha paradox and enables Sybil amplification.
    return pheromones[id].initialIntensity >> (elapsed / hl);
}

// CORRECT:
uint256 effectiveHL = hl / (1 + pheromones[id].confirmations);
```

### 3. DO NOT allow intensity to wrap around

```solidity
// WRONG -- unchecked subtraction could underflow.
function badDecay(uint256 intensity, uint256 decayAmount) pure returns (uint256) {
    return intensity - decayAmount;  // If decayAmount > intensity: UNDERFLOW
}
```

The contract avoids this by using right-shift (`>>`) and multiplication, which
can produce zero but never underflow.

### 4. DO NOT skip the existence check

```solidity
// WRONG -- a deleted/non-existent pheromone has depositBlock == 0.
// block.number - 0 = block.number, which would compute decay as if
// the pheromone was deposited at genesis. This returns a nonsensical
// positive intensity for a non-existent pheromone.
function badIntensity(uint256 id) view returns (uint256) {
    uint256 elapsed = block.number - pheromones[id].depositBlock;
    // ...
}

// CORRECT: check depositBlock != 0 first.
if (p.depositBlock == 0) return 0;
```

### 5. DO NOT let the depositor confirm their own pheromone

Self-confirmation is a free way to accelerate decay (or, if the alpha paradox
were inverted, to extend it). The contract requires `depositor != msg.sender`.

### 6. DO NOT allow double-confirmation

Without the `hasConfirmed` mapping, an agent could call `confirm()` repeatedly
to manipulate the half-life. Each call must be from a unique address.

---

## Checklist

Complete these items in order. Each builds on the previous.

- [ ] **Create contract file.** Create `PheromoneRegistry.sol` with the
      complete contract code from the "Copy-Paste Ready" section above.

- [ ] **Verify compilation.** Compile with `solc --optimize --optimize-runs 200`.
      Must compile cleanly with Solidity ^0.8.20, zero warnings.

- [ ] **Deploy to local testnet.** Deploy using Foundry (`forge create`) or
      Hardhat on a local anvil/hardhat node.

- [ ] **Write unit tests.** See test plan below. All tests must pass.

- [ ] **Gas profiling.** Run `forge test --gas-report` and verify gas costs
      match the estimates in this document (+/- 20%).

- [ ] **Review fixed-point math.** Verify that `currentIntensity` matches the
      worked examples in this document for at least 5 different elapsed/half-life
      combinations.

- [ ] **Review alpha paradox.** Verify that confirming a pheromone reduces its
      effective half-life and that the same pheromone decays faster after
      confirmation.

- [ ] **Review SINR.** Verify the SINR values for 1, 2, 10, and 100 interferers
      match the worked examples.

- [ ] **Review cleanup refunds.** Verify that cleanup of dead pheromones
      produces a net gas refund.

- [ ] **Review edge cases.** Test: deposit with intensity=100 (min), intensity=
      10000 (max), confirm by same address (should revert), confirm dead
      pheromone (should revert), cleanup of alive pheromone (should skip),
      cleanup of already-cleaned ID (should skip idempotently).

---

## Test Plan

### Setup

Each test deploys a fresh `PheromoneRegistry` and uses Foundry's cheat codes
(`vm.roll`, `vm.prank`) to control block numbers and sender addresses.

### Test Cases

#### 1. `test_deposit_basic`

```
1. Call deposit(vectorHash, 0, 1000) from address A.
2. Assert: nextId == 1.
3. Assert: pheromones[0].depositor == A.
4. Assert: pheromones[0].depositBlock == current block.
5. Assert: pheromones[0].initialIntensity == 1000.
6. Assert: pheromones[0].pheromoneType == 0.
7. Assert: pheromones[0].confirmations == 0.
8. Assert: PheromoneDeposited event was emitted with correct args.
```

#### 2. `test_deposit_reverts_on_invalid_type`

```
1. Call deposit(vectorHash, 3, 1000). Should revert "Invalid pheromone type".
```

#### 3. `test_deposit_reverts_on_intensity_bounds`

```
1. Call deposit(vectorHash, 0, 99).   Should revert "Intensity too low".
2. Call deposit(vectorHash, 0, 10001). Should revert "Intensity too high".
```

#### 4. `test_currentIntensity_no_decay`

```
1. Deposit with intensity=1000 at current block.
2. Assert: currentIntensity(0) == 1000 (no blocks elapsed).
```

#### 5. `test_currentIntensity_one_half_life`

```
1. Deposit THREAT (hl=100) with intensity=1000 at block N.
2. vm.roll(N + 100).
3. Assert: currentIntensity(0) == 500.
```

#### 6. `test_currentIntensity_two_half_lives`

```
1. Deposit THREAT with intensity=1000 at block N.
2. vm.roll(N + 200).
3. Assert: currentIntensity(0) == 250.
```

#### 7. `test_currentIntensity_death`

```
1. Deposit THREAT with intensity=1000 at block N.
2. vm.roll(N + 1000). (10 half-lives)
3. Assert: currentIntensity(0) == 0.  (1000 >> 10 = 0)
```

#### 8. `test_currentIntensity_fractional`

```
1. Deposit THREAT with intensity=1000 at block N.
2. vm.roll(N + 150).
3. q=1, r=50, k=4 -> FRAC_TABLE[4] = 707106781186547524
   Expected: (500 * 707106781186547524) / 1e18 = 353.
4. Assert: currentIntensity(0) == 353.
```

#### 9. `test_confirm_basic`

```
1. Deposit from address A.
2. Confirm from address B.
3. Assert: pheromones[0].confirmations == 1.
4. Assert: hasConfirmed[0][B] == true.
5. Assert: PheromoneConfirmed event emitted with confirmations=1,
           newHalfLife = HALF_LIVES[type] / 2.
```

#### 10. `test_confirm_accelerates_decay`

```
1. Deposit OPPORTUNITY (hl=250) with intensity=1000 at block N.
2. Confirm from B, C, D (3 confirmations).
   Effective HL = 250 / 4 = 62.
3. vm.roll(N + 62).
4. Assert: currentIntensity(0) == 500.  (exactly 1 effective half-life)
5. Without confirmations at +62 blocks, intensity would be ~917.
   This proves the alpha paradox works.
```

#### 11. `test_confirm_reverts_self`

```
1. Deposit from A.
2. Call confirm from A. Should revert "Cannot confirm own pheromone".
```

#### 12. `test_confirm_reverts_double`

```
1. Deposit from A.
2. Confirm from B. Success.
3. Confirm from B again. Should revert "Already confirmed".
```

#### 13. `test_confirm_reverts_dead`

```
1. Deposit THREAT with intensity=1000 at block N.
2. vm.roll(N + 2000). (intensity well below death threshold)
3. Confirm from B. Should revert "Pheromone is dead".
```

#### 14. `test_sinr_single`

```
1. Deposit with intensity=1000 at block N.
2. Call sinr(0, []) (no interferers).
3. Expected: 1000 * 1,000,000 / (0 + 10) = 100,000,000.
4. Assert: sinr == 100,000,000.
```

#### 15. `test_sinr_two_identical`

```
1. Deposit pheromone 0 with intensity=1000, type OPPORTUNITY.
2. Deposit pheromone 1 with intensity=1000, type OPPORTUNITY.
3. Call sinr(0, [1]).
4. Expected: 1000 * 1,000,000 / (1000 + 10) = 990,099.
5. Assert: sinr == 990099. (SINR ~ 0.99)
```

#### 16. `test_sinr_ten_identical`

```
1. Deposit 10 pheromones, all intensity=1000, same type.
2. Call sinr(0, [1,2,3,4,5,6,7,8,9]).
3. Expected: 1000 * 1,000,000 / (9000 + 10) = 110,987.
4. Assert: sinr == 110987. (SINR ~ 0.11)
```

#### 17. `test_sinr_hundred_identical`

```
1. Deposit 100 pheromones, all intensity=1000, same type.
2. Call sinr(0, [1..99]).
3. Expected: 1000 * 1,000,000 / (99,000 + 10) = 10,100.
4. Assert: sinr == 10100. (SINR ~ 0.01)
```

#### 18. `test_sinr_type_mismatch_reverts`

```
1. Deposit pheromone 0 as THREAT.
2. Deposit pheromone 1 as WISDOM.
3. Call sinr(0, [1]). Should revert "Interferer type mismatch".
```

#### 19. `test_cleanup_dead`

```
1. Deposit THREAT with intensity=1000 at block N.
2. vm.roll(N + 2000). (well past death)
3. Call cleanup([0]).
4. Assert: pheromones[0].depositBlock == 0 (deleted).
5. Assert: PheromoneCleaned event emitted.
```

#### 20. `test_cleanup_alive_skipped`

```
1. Deposit THREAT with intensity=1000 at block N.
2. vm.roll(N + 10). (still very alive)
3. Call cleanup([0]).
4. Assert: pheromones[0].depositBlock != 0 (NOT deleted).
5. Assert: no PheromoneCleaned event emitted.
```

#### 21. `test_cleanup_idempotent`

```
1. Deposit and wait until dead.
2. Call cleanup([0]). Success, emits event.
3. Call cleanup([0]) again. No revert, no event (idempotent skip).
```

#### 22. `test_all_pheromone_types_half_lives`

```
For each type in [THREAT, OPPORTUNITY, WISDOM]:
  1. Deposit with intensity=1000.
  2. Advance exactly HALF_LIVES[type] blocks.
  3. Assert: currentIntensity == 500.
```

#### 23. `test_gas_deposit`

```
1. Measure gas for deposit().
2. Assert: gas >= 60,000 && gas <= 100,000.
```

#### 24. `test_gas_confirm`

```
1. Measure gas for confirm().
2. Assert: gas >= 20,000 && gas <= 45,000.
```

---

## Audit Findings

**Audit date:** 2026-05-08
**Files audited:**
- `/Users/will/dev/nunchi/daeji/contracts/src/PheromoneRegistry.sol` (337 lines)
- `/Users/will/dev/nunchi/daeji/contracts/src/IPheromoneRegistry.sol` (86 lines)
- `/Users/will/dev/nunchi/daeji/contracts/test/PheromoneRegistry.t.sol` (412 lines)

### F-01: `deposit()` signature diverges from spec -- raw vector instead of hash

**Severity:** Design Deviation
**Location:** `PheromoneRegistry.sol` line 84-131, `IPheromoneRegistry.sol` line 53-57

The spec defines `deposit(bytes32 vectorHash, uint8 pheromoneType, uint16 intensity)`.
The implementation takes `deposit(bytes calldata location, PheromoneType pType, uint64 intensity)`.

Differences:
1. **`bytes calldata location` vs `bytes32 vectorHash`** -- The implementation accepts
   the full 1280-byte raw vector and hashes it on-chain (`keccak256(location)` at line
   99). The spec expects the caller to hash off-chain and pass only the 32-byte hash.
   The implementation's approach costs ~6x more calldata gas (1280 bytes vs 32 bytes).
   However, it does enable the contract to verify vector length (line 89), which is a
   reasonable tradeoff.
2. **`uint64 intensity` vs `uint16 intensity`** -- The implementation uses `uint64` for
   the intensity field (lines 48, 88). The spec uses `uint16`. Both enforce the same
   range (100-10000), so this has no functional impact, but wastes 6 bytes of storage
   per deposit.
3. **`PheromoneType pType` vs `uint8 pheromoneType`** -- The implementation uses the
   enum directly; the spec uses a raw `uint8`. The enum is safer (reverts on invalid
   values automatically).
4. **`payable` with `MIN_PHEROMONE_STAKE`** -- The implementation requires a minimum
   stake of 0.001 ether (line 18, 94-97). The spec has no staking requirement. This is
   an unadvertised economic mechanism.
5. **No return value** -- The spec returns `uint256 id`. The implementation returns
   nothing.

### F-02: ID generation uses content-addressable hash instead of auto-increment

**Severity:** Design Deviation
**Location:** `PheromoneRegistry.sol` lines 100-102

The spec uses a sequential `nextId++` counter (simple, predictable, no collisions).
The implementation uses `keccak256(abi.encodePacked(locationHash, msg.sender, block.number, uint8(pType)))`.

Consequences:
- Two deposits by the same sender at the same block with the same location and type
  will collide (guarded by the existence check at line 106, but the second deposit
  simply reverts rather than succeeding with a different ID).
- The spec's `uint256` key is replaced by `bytes32`, changing every downstream
  signature.
- The content-addressable scheme makes IDs unpredictable, which is fine for on-chain
  use but complicates off-chain indexing compared to sequential IDs.

### F-03: `confirm()` missing self-confirmation guard

**Severity:** Medium -- Security
**Location:** `PheromoneRegistry.sol` lines 135-169

The spec explicitly requires `require(p.depositor != msg.sender, "Cannot confirm own pheromone")` (spec line 262, anti-pattern #5 at spec line 974-977).

The implementation has NO such check. A depositor can confirm their own pheromone,
which allows a single address to deposit and immediately confirm in the same transaction,
artificially manipulating the half-life. Combined with Sybil accounts, this reduces
the cost of half-life manipulation.

### F-04: `confirm()` resets intensity baseline -- not in spec

**Severity:** Design Deviation -- Potentially Beneficial
**Location:** `PheromoneRegistry.sol` lines 151-159

On confirmation, the implementation snapshots the current decayed intensity (line 152),
stores it as the new `p.intensity` (line 158), and resets `p.depositBlock` to the
current block (line 159). This means decay restarts from the current intensity with
the shorter half-life.

The spec does NOT do this. The spec's `confirm()` (lines 679-696) only increments
`p.confirmations` and emits an event. Decay in the spec always uses `initialIntensity`
and the original `depositBlock`, with the effective half-life computed dynamically from
the current confirmation count.

The implementation's approach is arguably more correct for the alpha paradox (it prevents
a retroactive recalculation of past decay with the new half-life), but it diverges from
the spec.

### F-05: `sinr()` signature and logic differ significantly

**Severity:** Design Deviation
**Location:** `PheromoneRegistry.sol` lines 219-251, `IPheromoneRegistry.sol` line 77

**Spec:** `sinr(uint256 targetId, uint256[] calldata interfererIds)` -- caller provides
interferer IDs explicitly. The contract verifies type match and rejects self-interference.
`SINR_SCALE = 1_000_000`.

**Implementation:** `sinr(bytes32 pheromoneId)` -- takes only the target ID. Iterates
over ALL pheromones at the same `locationHash` (from `_locationPheromones[locHash]`,
lines 229-244), filtering by same type. `SINR_SCALE = 10_000` (line 33).

Consequences:
1. The implementation is O(n) in the number of pheromones at a location, making it a
   potential gas bomb for view calls. The spec's caller-provided-interferers approach
   is O(k) where k is the number of provided interferers.
2. The SINR scale differs by 100x: spec uses 1,000,000 (SINR 1.0 = 1,000,000), impl
   uses 10,000 (SINR 1.0 = 10,000). This changes all downstream SINR thresholds.
3. The spec's `require(iid != targetId)` and `require(pheromones[iid].pheromoneType == targetType)` enforcement is replaced by silent `continue` skips.

### F-06: `cleanup()` and `cleanupBatch()` split vs. spec's single batch function

**Severity:** Design Deviation -- Benign
**Location:** `PheromoneRegistry.sol` lines 172-204

The spec provides a single `cleanup(uint256[] calldata pheromoneIds)` function that
takes an array. The implementation provides both `cleanup(bytes32 pheromoneId)` (single)
and `cleanupBatch(bytes32[] calldata pheromoneIds)` (batch).

The single-item `cleanup()` reverts on alive pheromones (line 176-178), while the spec's
batch silently skips alive ones. The batch `cleanupBatch()` matches the spec's skip
behavior.

The `cleanup()` single-item variant also maintains a `_locationPheromones` index (line
183), which the spec does not have (the spec has no location-based index).

### F-07: Event names and shapes differ

**Severity:** Design Deviation
**Location:** `IPheromoneRegistry.sol` lines 25-46

| Spec Event | Impl Event | Differences |
|-----------|-----------|-------------|
| `PheromoneDeposited(uint256 id, address depositor, uint8 type, uint16 intensity)` | `PheromoneDeposited(bytes32 id, bytes32 locationHash, address depositor, uint8 type, uint64 intensity, uint64 depositBlock)` | Extra fields: `locationHash`, `depositBlock`. Type widths differ. ID is `bytes32` not `uint256`. |
| `PheromoneConfirmed(uint256 id, uint32 confirmations, uint64 newHalfLife)` | `PheromoneConfirmed(bytes32 id, address confirmer, uint64 newConfirmationCount, uint64 newEffectiveHalfLife)` | Extra `confirmer` field. `confirmationCount` type is `uint64` vs `uint32`. `confirmer` is `indexed`. |
| `PheromoneCleaned(uint256 id)` | `PheromonePruned(bytes32 id, address pruner)` | Renamed. Extra `pruner` field. Both are `indexed`. |

The implementation events carry more information (generally good for off-chain
indexing), but any existing tooling built against the spec events will break.

### F-08: `readPheromones()` is a stub

**Severity:** High -- Incomplete
**Location:** `PheromoneRegistry.sol` lines 254-263

The function body is:
```solidity
ids = new bytes32[](0);
sinrValues = new uint64[](0);
```

This always returns empty arrays. The comment says "Placeholder: full implementation
requires precompile search." This function exists in the interface
(`IPheromoneRegistry.sol` line 81-85) but is never functional.

The spec does not define a `readPheromones()` function -- this was added by the
implementation. It is declared in the interface as a required function but is
completely non-functional.

### F-09: `_locationPheromones` index grows unboundedly

**Severity:** Medium -- Gas/DoS
**Location:** `PheromoneRegistry.sol` lines 62, 120, 229-244

Every `deposit()` pushes into `_locationPheromones[locationHash]` (line 120). The
`cleanup()` function removes entries via swap-and-pop (lines 324-336). However:

1. If cleanup is not called regularly, the array grows without bound.
2. The `sinr()` view function iterates the entire array (lines 232-244). A location
   with thousands of deposits (live or dead-but-uncleaned) will cause `sinr()` to
   consume excessive gas, even as a view call via `eth_call`.
3. Dead-but-uncleaned entries in the array still cost iteration gas in `sinr()`. The
   `other.depositBlock == 0` check at line 237 only skips pruned entries; entries that
   are dead but not yet cleaned are still iterated and have `_computeIntensity()` called
   on them.

### F-10: Deposited ETH is permanently locked

**Severity:** High -- Funds at Risk
**Location:** `PheromoneRegistry.sol` lines 18, 94-97

The contract requires `msg.value >= MIN_PHEROMONE_STAKE` (0.001 ether) on each deposit.
There is no `withdraw()`, no `receive()` reverting guard, no refund mechanism, and no
forwarding of funds. All deposited ETH is permanently locked in the contract with no
way to retrieve it.

The spec has no staking requirement, so this is entirely an implementation addition
with no corresponding withdrawal mechanism.

### F-11: Decay math uses linear interpolation, spec uses lookup table

**Severity:** Low -- Precision
**Location:** `PheromoneRegistry.sol` lines 289-316

The spec's "Copy-Paste Ready" contract uses a `FRAC_TABLE[8]` lookup table with
precomputed `2^(-k/8)` values (maximum error ~0.5%). The implementation uses linear
interpolation: `(2*hl - r) / (2*hl)` (lines 308-309), which has maximum error ~5% at
r ~ 0.33*hl. The spec explicitly discusses the linear interpolation as a lower-precision
alternative and recommends the lookup table.

The implementation also uses `FP_SCALE = 65536` (2^16) for fixed-point scaling (line 39)
instead of the spec's `PRECISION = 1e18`. This is sufficient for the value range but
provides less fractional precision.

### F-12: `HdcLib` imported but never used

**Severity:** Low -- Code Smell
**Location:** `PheromoneRegistry.sol` line 5

`import {HdcLib} from "./HdcPrecompile.sol"` is declared but `HdcLib` is never
referenced anywhere in the contract. This violates the spec's statement that the
contract is "self-contained" and "does NOT depend on... the HDC precompile."

The `PROXIMITY_THRESHOLD` constant (line 36) suggests planned use of `HdcLib.hamming()`
for proximity checks, but this is not implemented.

### F-13: `_confirmers` mapping is not publicly readable

**Severity:** Low -- Transparency
**Location:** `PheromoneRegistry.sol` line 65

The spec declares `mapping(uint256 => mapping(address => bool)) public hasConfirmed`
(spec line 174). The implementation declares it as `internal`:
`mapping(bytes32 => mapping(address => bool)) internal _confirmers`.

There is no getter function. External callers cannot check whether a given address has
already confirmed a specific pheromone without calling `confirm()` and catching the
revert.

---

## Implementation Status

| Spec Feature | Status | Notes |
|-------------|--------|-------|
| `deposit()` | Partial | Signature differs (raw vector vs hash, payable, no return). Core logic works. |
| `confirm()` | Partial | Missing self-confirmation guard (F-03). Added intensity reset (F-04). |
| `currentIntensity()` | Implemented | Lower precision than spec (linear interp vs lookup table, F-11). Correct at half-life boundaries. |
| `sinr()` | Diverged | Different signature, auto-discovers interferers, different scale (F-05). |
| `cleanup()` | Implemented | Split into single + batch. Single reverts on alive; batch skips. |
| `readPheromones()` | Stub | Returns empty arrays. Not in spec. Interface declares it but body is empty (F-08). |
| Sequential IDs (`nextId`) | Not implemented | Replaced with content-addressable hashing (F-02). |
| `hasConfirmed` public getter | Not implemented | Mapped as `internal _confirmers` (F-13). |
| `FRAC_TABLE` lookup | Not implemented | Using linear interpolation instead (F-11). |
| `_locationPheromones` index | Added | Not in spec. Enables location-based `sinr()` auto-discovery. |
| Staking mechanism | Added | 0.001 ETH per deposit, no withdrawal (F-10). |
| `PheromoneType` enum in interface | Added | Spec puts enum in contract; impl puts it in a separate `IPheromoneRegistry`. |
| Event schemas | Diverged | Richer events with extra fields and different names (F-07). |

---

## Anti-Patterns & Duct Tape

### A-01: Stub function in interface

`readPheromones()` at `PheromoneRegistry.sol:254-263` is declared in the interface
(`IPheromoneRegistry.sol:81-85`) but always returns empty arrays. This means any
contract that calls `IPheromoneRegistry.readPheromones()` will silently get no results
with no error. A function that cannot work should either revert with
`"not implemented"` or not be in the interface at all.

### A-02: Unused import

`import {HdcLib} from "./HdcPrecompile.sol"` at `PheromoneRegistry.sol:5` introduces a
dependency on the precompile library that is never called. The spec explicitly states
the contract is self-contained. This import should be removed.

### A-03: Magic number for vector size

`require(location.length == 1280, ...)` at `PheromoneRegistry.sol:89` uses a hardcoded
`1280`. This should be a named constant (e.g., `uint256 constant VECTOR_SIZE = 1280`).
The constant `PROXIMITY_THRESHOLD = 1024` at line 36 also appears to be dead code
(never referenced in any function).

### A-04: Unreachable overflow guard

`if (result > type(uint64).max) return type(uint64).max` at `PheromoneRegistry.sol:314`.
Given that `integerDecay <= MAX_INTENSITY = 10_000` and `numerator <= 2*hl`, the
intermediate `uint128` value can never exceed `uint64` max (`~1.8e19`). The guard is
defensive but unreachable.

### A-05: String revert messages instead of custom errors

All `require()` calls use string messages (`PheromoneRegistry.sol` lines 89, 92, 96,
106, 137, 139, 145, 174, 176). Since Solidity 0.8.20 is targeted, custom errors
(`error PheromoneRegistry__InvalidVectorSize()`) would save deployment gas and calldata
gas on reverts.

### A-06: Swap-and-pop in `_removeFromLocationIndex` is O(n)

`PheromoneRegistry.sol:324-336`. The swap-and-pop itself is O(1), but finding the
element to swap requires an O(n) linear scan. For locations with many pheromones, this
is expensive. A mapping from `pheromoneId -> index` would make removal O(1).

---

## Security Concerns

### S-01: Missing self-confirmation guard (Critical)

**File:** `PheromoneRegistry.sol`, `confirm()` function, lines 135-169.

The depositor can confirm their own pheromone. This violates spec anti-pattern #5
(spec line 974-977). Impact: a depositor can deposit and confirm in one transaction,
reducing the half-life. While the alpha paradox means this accelerates decay (which
is self-harming), it still enables manipulation:
- An attacker deposits with max intensity, immediately self-confirms multiple times
  (each from a different contract/address within a single tx via a factory), and uses
  the resulting short half-life to grief the SINR of legitimate pheromones at the same
  location before the attacker's own signal dies.

### S-02: Permanently locked ETH (High)

**File:** `PheromoneRegistry.sol`, lines 18, 94-97.

Every `deposit()` requires >= 0.001 ETH. The contract has no `withdraw()`, no
`selfdestruct` (deprecated), and no forwarding logic. ETH accumulates with no exit
path. Over time, the contract becomes a permanent ETH sink. If staking is intended,
there must be a slashing/refund mechanism tied to pheromone lifecycle (e.g., refund
on cleanup after the pheromone dies naturally).

### S-03: DoS via `sinr()` gas exhaustion

**File:** `PheromoneRegistry.sol`, lines 219-251.

An attacker can deposit thousands of pheromones at the same `locationHash` (cost:
1000 * 0.001 ETH = 1 ETH). The `sinr()` function iterates every entry in
`_locationPheromones[locHash]`. Even as a `view` call, this can hit the block gas
limit if the array is large enough, making `sinr()` uncallable for that location.
If any on-chain contract depends on `sinr()`, this is a DoS vector.

### S-04: Pheromone ID collision within same block

**File:** `PheromoneRegistry.sol`, lines 100-107.

The pheromone ID is `keccak256(locationHash, msg.sender, block.number, uint8(pType))`.
If the same sender deposits the same type at the same location in the same block, the
ID collides. The existence check at line 106 causes a revert. This is not exploitable
(it only harms the sender), but it limits throughput: a single address can deposit at
most 3 pheromones per block per location (one per type).

### S-05: No access control on `cleanup()`

**File:** `PheromoneRegistry.sol`, lines 172-204.

Anyone can call `cleanup()` or `cleanupBatch()` on any dead pheromone. This is by
design (the spec says "Anyone can call this"), but combined with the staking mechanism
(which is NOT in the spec), it means the depositor's stake is burned and the cleaner
gets the SSTORE refund gas. The economic incentives are misaligned: the depositor pays
but a third party profits from cleanup.

### S-06: `cleanupBatch()` unbounded loop

**File:** `PheromoneRegistry.sol`, lines 192-204.

No limit on `pheromoneIds.length`. A caller could pass a very large array, though this
is bounded by calldata cost and block gas limit. The `_removeFromLocationIndex` call
inside the loop (line 200) is itself O(n) in the location array size, making the
worst case O(n*m) where n = batch size and m = location array size.

---

## Recommended Changes Checklist

- [ ] **[Critical] Add self-confirmation guard.** In `confirm()` at
      `PheromoneRegistry.sol:135`, add:
      `require(p.depositor != msg.sender, "PheromoneRegistry: cannot self-confirm");`
      after the existence check.

- [ ] **[High] Fix locked ETH.** Either (a) remove the staking requirement entirely
      (aligning with spec), or (b) add a `withdrawStake(bytes32 pheromoneId)` function
      that refunds the depositor's stake after the pheromone has died and been cleaned,
      or (c) forward the stake to a treasury/DAO address on deposit.

- [ ] **[High] Implement or remove `readPheromones()`.** Either implement the function
      using `HdcLib.searchSimilar()` or remove it from both the interface and the
      contract. A stub that silently returns empty data is a footgun for callers.

- [ ] **[Medium] Bound `_locationPheromones` array or add pagination to `sinr()`.**
      Options: (a) cap deposits per location, (b) add a `maxIter` parameter to
      `sinr()`, (c) switch to the spec's caller-provides-interferers design.

- [ ] **[Medium] Make `_confirmers` public or add a getter.** Add
      `function hasConfirmed(bytes32 pheromoneId, address account) external view returns (bool)`
      so callers can check confirmation status before attempting to confirm.

- [ ] **[Low] Remove unused `HdcLib` import.** Delete line 5 of
      `PheromoneRegistry.sol`. Also remove the unused `PROXIMITY_THRESHOLD` constant
      at line 36.

- [ ] **[Low] Replace hardcoded `1280` with a named constant.** Add
      `uint256 public constant VECTOR_SIZE = 1280;` and use it in the `deposit()`
      require.

- [ ] **[Low] Consider using the spec's lookup-table decay.** Replace linear
      interpolation (`_computeIntensity`, lines 289-316) with the `FRAC_TABLE[8]`
      approach from the spec to reduce maximum error from ~5% to ~0.5%.

- [ ] **[Low] Use custom errors instead of string reverts.** Replace all
      `require(..., "string")` with custom `error` declarations for gas savings.

- [ ] **[Low] Align `SINR_SCALE` with spec.** Change from `10_000` to `1_000_000` or
      document the intentional deviation. The 100x difference silently breaks any
      threshold logic calibrated to the spec's scale.

- [ ] **[Test] Add test: depositor self-confirms.** Verify `confirm()` reverts when
      called by the depositor. (Currently untested and will pass -- which is the bug.)

- [ ] **[Test] Add test: `readPheromones()` returns empty.** Document that this is a
      known stub, or test that it reverts.

- [ ] **[Test] Add test: SINR with 10+ interferers at same location.** Stress-test the
      `_locationPheromones` iteration path.

- [ ] **[Test] Add test: deposit same location/type/sender in same block reverts.**
      Verify the ID collision guard works.

- [ ] **[Test] Add test: multiple confirmations from different addresses.** Test with
      4+ confirmers to verify effective half-life = base / (1 + N) for larger N.

- [ ] **[Test] Add test: cleanup refunds depositor or verify ETH is trapped.** If
      staking is kept, test the ETH flow.

- [ ] **[Test] Add gas profiling tests.** The spec requires `test_gas_deposit` and
      `test_gas_confirm` (spec tests 23-24). Neither is implemented.

- [ ] **[Test] Add fractional decay accuracy test at r = 1.5 half-lives (150 blocks).**
      Spec test case 8 (`test_currentIntensity_fractional`) expects 353 with the lookup
      table. The implementation's linear interpolation will produce a different value.
      The current tests only check exact half-life boundaries where both methods agree.

---

## Second-Pass Remediation Detail

This pass assumes the current implementation shape in
`contracts/src/PheromoneRegistry.sol`, `contracts/src/IPheromoneRegistry.sol`, and
`contracts/src/HdcPrecompile.sol`, not the older copy-paste-ready spec above. The
items below are concrete remediation targets for the implementation, interface, and
tests.

### R-01: Self-confirmation guard

Current `confirm(bytes32 pheromoneId)` prevents duplicate confirmations but still lets
the original depositor confirm their own pheromone.

Concrete fix:

```solidity
function confirm(bytes32 pheromoneId) external {
    PheromoneDeposit storage p = pheromones[pheromoneId];
    require(p.depositBlock != 0, "PheromoneRegistry: does not exist");
    require(
        p.depositor != msg.sender,
        "PheromoneRegistry: cannot self-confirm"
    );
    require(
        !_confirmers[pheromoneId][msg.sender],
        "PheromoneRegistry: already confirmed"
    );
    ...
}
```

Place the guard immediately after the existence check and before `_confirmers` is
written. Keep the existing duplicate confirmer guard. Add a public
`hasConfirmed(bytes32 pheromoneId, address account)` getter if clients need to avoid
probing by revert.

### R-02: `readPheromones()` implementation

Current `readPheromones()` always returns two empty arrays. This is worse than a
revert because callers can interpret "no pheromones" as a valid result.

Concrete fix, if the function remains in the interface:

1. Add constants:

```solidity
uint256 public constant VECTOR_SIZE = 1280;
uint8 public constant MAX_TOP_K = 32;
uint8 public constant MAX_SEARCH_CANDIDATES = 128;
```

2. In `deposit()`, store the vector in the HDC precompile under the final pheromone ID:

```solidity
HdcLib.storeVector(pheromoneId, location);
```

3. In both cleanup paths, remove the vector from the precompile before or after deleting
   contract storage:

```solidity
HdcLib.deleteVector(pheromoneId);
```

4. Implement `readPheromones()` as a bounded HDC search:

```solidity
require(queryVector.length == VECTOR_SIZE, "PheromoneRegistry: invalid vector size");
require(topK > 0 && topK <= MAX_TOP_K, "PheromoneRegistry: invalid topK");

uint8 candidateLimit = topK * 4;
if (candidateLimit < topK) candidateLimit = MAX_SEARCH_CANDIDATES; // overflow clamp
if (candidateLimit > MAX_SEARCH_CANDIDATES) candidateLimit = MAX_SEARCH_CANDIDATES;

(bytes32[] memory candidateIds, uint16[] memory distances) =
    HdcLib.searchSimilar(queryVector, candidateLimit);
```

Then filter in memory:

- `distance <= PROXIMITY_THRESHOLD`
- `pheromones[id].depositBlock != 0`
- `pheromones[id].pType == uint8(pType)`
- `_computeIntensity(pheromones[id]) >= DEATH_THRESHOLD`
- output length never exceeds `topK`

Compute SINR against only the bounded candidate set, not the entire location index. If
the precompile cannot support a persistent vector index in production, remove
`readPheromones()` from `IPheromoneRegistry` and make callers use an off-chain index.
Do not keep a silent empty stub.

### R-03: Deposits, withdrawals, and trapped Ether

Current deposits require `msg.value >= MIN_PHEROMONE_STAKE`, but the contract has no
refund, withdrawal, slashing, treasury, or accounting path. This permanently traps
Ether sent through `deposit()`.

Choose one policy and encode it explicitly:

**Option A: no staking.** Remove `payable`, remove `MIN_PHEROMONE_STAKE`, and reject
accidental Ether:

```solidity
receive() external payable {
    revert("PheromoneRegistry: direct ETH not accepted");
}
```

Forced Ether can still arrive via protocol mechanisms, so do not rely on
`address(this).balance == accountedStake`.

**Option B: refundable anti-spam stake.** Record the exact stake per pheromone and use
the withdrawal pattern for payouts:

```solidity
mapping(bytes32 => uint256) public stakeOf;
mapping(address => uint256) public pendingWithdrawals;

function deposit(...) external payable {
    require(msg.value >= MIN_PHEROMONE_STAKE, "PheromoneRegistry: insufficient stake");
    ...
    stakeOf[pheromoneId] = msg.value;
}

function _queueStakeRefund(bytes32 pheromoneId, address depositor) internal {
    uint256 amount = stakeOf[pheromoneId];
    delete stakeOf[pheromoneId];
    if (amount != 0) pendingWithdrawals[depositor] += amount;
}

function withdraw() external {
    uint256 amount = pendingWithdrawals[msg.sender];
    require(amount != 0, "PheromoneRegistry: nothing to withdraw");
    pendingWithdrawals[msg.sender] = 0;
    (bool ok, ) = payable(msg.sender).call{value: amount}("");
    require(ok, "PheromoneRegistry: withdraw failed");
}
```

Queue the refund during cleanup after taking a local copy of `p.depositor`. Do not send
Ether from `cleanup()`; the cleaner should not be able to block cleanup by making the
depositor's receive path fail.

Relevant external reference: Solidity's common patterns documentation recommends the
withdrawal pattern for Ether payouts, and the security considerations describe why
external Ether transfers hand control to the recipient. See
`https://docs.solidity.org/en/latest/common-patterns.html#withdrawal-from-contracts`
and
`https://docs.solidity.org/en/latest/security-considerations.html#sending-and-receiving-ether`.

### R-04: Bounded and enumerable location indexes

Current `_locationPheromones[locationHash]` grows without a hard cap, and
`_removeFromLocationIndex()` performs a linear scan before swap-and-pop.

Concrete fix:

```solidity
uint256 public constant MAX_PHEROMONES_PER_LOCATION = 256;
uint256 public constant MAX_CLEANUP_BATCH = 64;

mapping(bytes32 => bytes32[]) internal _locationPheromones;
mapping(bytes32 => mapping(bytes32 => uint256)) internal _locationIndexPlusOne;
```

On deposit:

```solidity
bytes32[] storage arr = _locationPheromones[locationHash];
require(arr.length < MAX_PHEROMONES_PER_LOCATION, "PheromoneRegistry: location full");
arr.push(pheromoneId);
_locationIndexPlusOne[locationHash][pheromoneId] = arr.length;
```

On removal:

```solidity
uint256 indexPlusOne = _locationIndexPlusOne[locationHash][pheromoneId];
if (indexPlusOne == 0) return;

uint256 index = indexPlusOne - 1;
bytes32[] storage arr = _locationPheromones[locationHash];
bytes32 moved = arr[arr.length - 1];

arr[index] = moved;
_locationIndexPlusOne[locationHash][moved] = index + 1;
arr.pop();
delete _locationIndexPlusOne[locationHash][pheromoneId];
```

Also cap `cleanupBatch()` with `require(pheromoneIds.length <= MAX_CLEANUP_BATCH, ...)`.

Relevant external reference: Solidity mappings do not store keys, have no length, and
cannot be enumerated or fully cleaned without separately tracking assigned keys. See
`https://docs.solidity.org/en/latest/types.html#mapping-types`. This means any
mapping-backed cleanup or pruning design must maintain companion arrays, enumerable
sets, or index-plus-one maps.

### R-05: SINR complexity

Current `sinr(bytes32 pheromoneId)` scans every pheromone at the exact same
`locationHash`. A single hot location can make the function unusable.

Concrete fix:

- Deprecate the unbounded auto-discovery path, or keep it only when
  `_locationPheromones[locHash].length <= MAX_PHEROMONES_PER_LOCATION`.
- Add a bounded computation path:

```solidity
uint8 public constant MAX_INTERFERERS = 64;

function sinr(
    bytes32 targetId,
    bytes32[] calldata interfererIds
) external view returns (uint64) {
    require(interfererIds.length <= MAX_INTERFERERS, "PheromoneRegistry: too many interferers");
    return _sinrFromCandidates(targetId, interfererIds);
}
```

- In `_sinrFromCandidates`, require each interferer to exist, reject `targetId`, require
  matching `pType`, and either require matching `locationHash` or require that the
  candidate set came from bounded HDC search in `readPheromones()`.
- Use `uint256` for the interference accumulator and clamp only at the final `uint64`
  return if the interface keeps `uint64`.
- Align or explicitly document `SINR_SCALE`. The current implementation uses `10_000`;
  the spec examples use `1_000_000`. This must be a deliberate ABI-level choice because
  downstream thresholds depend on it.

### R-06: Pheromone ID scheme

Current IDs are `keccak256(locationHash, msg.sender, block.number, pType)`, so the same
sender cannot deposit the same type at the same location twice in one block.

Concrete fix:

```solidity
uint64 public nextPheromoneNonce = 1;

function _nextPheromoneId(
    bytes32 locationHash,
    address depositor,
    PheromoneType pType
) internal returns (bytes32 pheromoneId) {
    uint64 nonce = nextPheromoneNonce++;
    pheromoneId = keccak256(
        abi.encode(
            block.chainid,
            address(this),
            nonce,
            locationHash,
            depositor,
            uint8(pType)
        )
    );
}
```

Use `abi.encode`, not `abi.encodePacked`, for the ID preimage. Return the ID from
`deposit()` or emit enough event data for callers to discover it deterministically. If
duplicate suppression is desired, implement it separately with a
`latestByLocationDepositorType` mapping and an explicit policy instead of making ID
collisions the control flow.

### R-07: Decay math

Current decay uses a linear approximation and resets `intensity` plus `depositBlock`
on confirmation. The reset is defensible because it avoids retroactively applying the
new shorter half-life to old time, but the math and limits should be made explicit.

Concrete fix:

- Choose and document one fractional method:
  - keep linear interpolation and state the approximate maximum error in NatSpec and
    tests, or
  - switch to the spec's `FRAC_TABLE` lookup to reduce error.
- Remove the unused `FP_SCALE` constant if linear math remains.
- Cap the effective half-life so excessive confirmations do not collapse to zero
  unless that is intended:

```solidity
uint64 public constant MIN_EFFECTIVE_HALF_LIFE = 1;

uint64 hl = base / (1 + uint64(p.confirmationCount));
if (hl < MIN_EFFECTIVE_HALF_LIFE) hl = MIN_EFFECTIVE_HALF_LIFE;
```

- Consider widening `confirmationCount` to `uint32` or define a maximum confirmation
  count and revert before overflow.
- Use `uint256` intermediates for decay and SINR calculations. Return `0` once the
  integer halving count guarantees death; avoid unreachable saturation branches.
- Add tests at exact half-life boundaries and fractional points (`r = hl / 2`,
  `r = hl - 1`, and after confirmation resets the baseline).

### R-08: Event schema

Current events are more useful than the original spec but still omit stake movement and
do not make all lifecycle transitions observable.

Concrete fix:

```solidity
event PheromoneDeposited(
    bytes32 indexed pheromoneId,
    bytes32 indexed locationHash,
    address indexed depositor,
    uint8 pType,
    uint64 intensity,
    uint256 stake,
    uint64 depositBlock,
    uint64 nonce
);

event PheromoneConfirmed(
    bytes32 indexed pheromoneId,
    address indexed confirmer,
    uint32 confirmationCount,
    uint64 priorIntensity,
    uint64 newBaselineIntensity,
    uint64 newEffectiveHalfLife,
    uint64 confirmBlock
);

event PheromonePruned(
    bytes32 indexed pheromoneId,
    address indexed pruner,
    address indexed depositor,
    uint256 queuedRefund,
    uint64 pruneBlock
);

event StakeWithdrawn(address indexed account, uint256 amount);
```

Keep at most three indexed parameters per event. Treat any event shape change as an ABI
breaking change for indexers; if compatibility matters, add new events instead of
mutating the existing ones.

### R-09: Tests to add or update

Add tests that lock in the remediation behavior:

- `test_confirm_selfReverts`: depositor calling `confirm()` reverts with the new guard.
- `test_confirm_hasConfirmedGetter`: getter reports false before confirm and true after.
- `test_deposit_twoSameBlockSameSenderSameLocationSameTypeHaveDifferentIds`: validates
  the nonce-based ID scheme.
- `test_deposit_stakeRecordedExactly`: full `msg.value` is recorded, not only the
  minimum stake.
- `test_cleanup_queuesRefund`: cleanup deletes pheromone state and queues the
  depositor's pending withdrawal.
- `test_withdraw_usesPendingBalanceAndZerosBeforeCall`: pending balance is zeroed before
  Ether transfer; include a malicious receiver test if a test helper exists.
- `test_directEthRejected`: plain ETH transfer reverts, while acknowledging forced ETH
  is not preventable.
- `test_readPheromones_invalidVectorReverts`, `test_readPheromones_invalidTopKReverts`,
  and `test_readPheromones_returnsLiveNearbyTypeOnly`.
- `test_readPheromones_filtersDeadAndPruned`: dead or cleaned pheromones do not appear.
- `test_sinr_rejectsTooManyInterferers`: bounded path enforces `MAX_INTERFERERS`.
- `test_sinr_rejectsSelfInterferer` and `test_sinr_rejectsTypeMismatch`.
- `test_locationIndex_swapAndPopUpdatesMovedIndex`: cleanup keeps index-plus-one maps
  coherent after moving the last ID.
- `test_locationIndex_locationFullReverts`: deposit cap is enforced.
- `test_cleanupBatch_tooLargeReverts`: batch size cap is enforced.
- `test_decay_fractionalAccuracy`: expected result matches the chosen linear or lookup
  method at non-boundary blocks.
- `test_decay_confirmationDoesNotRetroactivelyRepriceHistory`: after a confirmation,
  current intensity is preserved and future decay uses the shorter half-life.
- `test_events_emitLifecycleFields`: deposit, confirm, prune, and withdraw events expose
  the fields indexers need.

These tests should replace the current assumption that `readPheromones()` can be empty
and should update SINR expectations if `SINR_SCALE` changes.

---

## Reconciliation: Solidity vs Rust Precompile

### What happened

This document specifies a Solidity `PheromoneRegistry.sol` contract. PR #42
implemented stigmergy as a **Rust precompile** (`StigmergyPrecompile`) at
address `0xA0D` instead. The two approaches diverge significantly:

| Aspect | This Doc (Solidity) | PR #42 (Rust Precompile) |
|--------|--------------------|-----------------------|
| Language | Solidity smart contract | Rust precompile at `0xA0D` |
| Alpha paradox | Yes -- `effective_half_life = base / (1 + confs)` | **No** -- monotonic pheromones only |
| SINR interference | Full SINR with noise floor | Not implemented |
| Decay model | Fixed-point integer, lazy eval at read time | Event-driven state, unclear decay path |
| Cleanup/eviction | Explicit `cleanup()` with storage refunds | Not implemented |
| Storage | EVM storage slots (Solidity mappings) | In-memory Rust data structures |

### Key files (PR #42 Rust precompile)

- Precompile registration: look for `StigmergyPrecompile` in the precompile registry
- Event definitions: `/Users/will/dev/nunchi/daeji/crates/hdc/chain/src/event.rs` -- includes `PheromoneDeposited` event decoder
- Index stub: `/Users/will/dev/nunchi/daeji/crates/hdc/chain/src/index.rs` line ~130 -- `record_pheromone()` is a TODO stub

### Decision required

- [ ] **Decide: keep Solidity contract OR use Rust precompile (PR #42)**
  - Solidity: more portable, auditable, standard tooling (Foundry tests)
  - Rust precompile: faster execution, no EVM overhead, tighter chain integration
  - Recommendation: Rust precompile is already partially wired; finish it

### Remaining stigmergy work (if Rust precompile path is chosen)

- [ ] Implement alpha paradox in `StigmergyPrecompile` -- `effective_half_life = base_half_life / (1 + confirmation_count)`
- [ ] Implement SINR interference model -- `sinr = signal * SCALE / (interference + NOISE_FLOOR)` per this doc's spec
- [ ] Implement fixed-point decay in the precompile (integer-only, no floats) -- port `currentIntensity` logic from this doc
- [ ] Implement cleanup/eviction at `7x effective_half_life` -- remove dead pheromones from storage
- [ ] Implement `record_pheromone()` backing storage in `/Users/will/dev/nunchi/daeji/crates/hdc/chain/src/index.rs` (currently a TODO stub)
- [ ] Wire pheromone event decoding in `/Users/will/dev/nunchi/daeji/crates/hdc/chain/src/event.rs` -- `process_log()` does not ABI-decode `PheromoneDeposited`
- [ ] Add stigmergy E2E tests -- verify deposit, confirm, decay, SINR, and cleanup via precompile calls
- [ ] Add unit tests for integer decay math (port worked examples from this doc as test vectors)

### Verification commands

```bash
# Check that the precompile module compiles
cargo check -p kora-hdc-chain

# Run existing chain-level tests
cargo test -p kora-hdc-chain

# Search for pheromone-related stubs
grep -rn "TODO.*pheromone\|record_pheromone\|StigmergyPrecompile" crates/hdc/
```
