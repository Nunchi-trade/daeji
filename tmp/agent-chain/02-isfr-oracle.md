# ISFR Oracle — Requirements & Implementation

Precompile address: `0xA01`

The Implied Secured Funding Rate is a composite benchmark index derived from
on-chain yield sources. It serves the same role for DeFi that SOFR serves for
traditional finance — a risk-free reference rate that prices everything else.

---

## What ISFR Is

A single number (annual rate in basis points) computed from four classes of
on-chain yield:

| Class | Weight | Sources | Rationale |
|-------|--------|---------|-----------|
| LENDING | 60% | Aave, Compound, Morpho supply rates | Largest, most liquid, most stable |
| STRUCTURED | 25% | Pendle PT yields, vault strategies | Represents structured product yields |
| FUNDING | 10% | Perp funding rates (dYdX, Hyperliquid) | Market sentiment component |
| STAKING | 5% | ETH staking yield, LST rates | Base layer yield floor |

### Aggregation Pipeline

```
Source rates (per class)
  → Outlier filter (reject > 3σ from class median)
  → TVL-weighted median (intra-class)
  → Class rates [4 values]
  → Weight application (60/25/10/5)
  → Weighted sum
  → Final ISFR rate (basis points, uint64)
```

### Two-Level Byzantine Aggregation

For a validator-run oracle:

1. **Intra-class:** Each validator computes TVL-weighted median per class from its own source observations
2. **Inter-validator:** Stake-weighted median across all validators' class rates
3. **Final:** Weighted sum of consensus class rates

This tolerates up to 1/3 Byzantine validators (matching BFT assumption).

---

## Precompile Interface

| Selector | Method | Gas | Description |
|----------|--------|-----|-------------|
| `0x01` | `current()` | 2,100 | Latest ISFR rate (basis points) + block height |
| `0x02` | `currentRate()` | 2,100 | Latest rate only (uint64) |
| `0x03` | `at(uint64 blockHeight)` | 5,000 | Historical rate at specific block |
| `0x04` | `twap(uint64 startBlock, uint64 endBlock)` | 10,000 | Time-weighted average over range |
| `0x05` | `history(uint64 count)` | 5,000 + 500/entry | Last N published rates |
| `0x06` | `classRates()` | 3,000 | Current rates per class (4 values) |
| `0x07` | `circuitBreakerState()` | 2,100 | Current circuit breaker status |

### Return Types

```solidity
// current() returns:
struct ISFRSnapshot {
    uint64 rate;        // basis points (e.g., 325 = 3.25%)
    uint64 blockHeight; // block at which this rate was published
    uint64 timestamp;   // block timestamp
    uint8 state;        // circuit breaker state (0=Live, 1=Degraded, 2=Stale, 3=Halted)
}

// classRates() returns:
struct ClassRates {
    uint64 lending;     // 60% weight
    uint64 structured;  // 25% weight
    uint64 funding;     // 10% weight
    uint64 staking;     // 5% weight
}
```

---

## Publication Cadence

- **Frequency:** Every ~25 blocks (~10 seconds at 400ms block time)
- **Mechanism:** Validators include ISFR observations in their block proposals
- **Storage:** Ring buffer of last 8,640 snapshots (~24 hours at 10s intervals)
- **History access:** Older rates available via QMDB state queries

### Block Proposal Integration

```
Proposer builds block:
  1. Collect source rates from oracle feeds
  2. Compute local class rates (TVL-weighted medians)
  3. Include ISFRObservation in block header extension

Validators verify block:
  1. Compute own class rates
  2. Compare with proposer's observation
  3. Accept if within tolerance (±50 bps per class)
  4. Reject if divergent (prevents manipulation)

Every 25th block:
  1. Aggregate last 25 observations (median)
  2. Publish to ISFR precompile storage
  3. Update circuit breaker state
```

---

## Circuit Breaker

Four states with automatic transitions:

```
┌──────┐   sources < 3    ┌──────────┐   sources < 1    ┌───────┐   rate = 0    ┌────────┐
│ LIVE │ ───────────────→ │ DEGRADED │ ───────────────→ │ STALE │ ──────────→  │ HALTED │
│      │ ← ─ ─ ─ ─ ─ ─ ─ │          │ ← ─ ─ ─ ─ ─ ─ ─│       │ ← ─ ─ ─ ─ ─ │        │
└──────┘   sources ≥ 3    └──────────┘   sources ≥ 1    └───────┘   rate > 0   └────────┘
             for 5 min                      for 2 min                  for 10 min
```

| State | Behavior | Consumer Impact |
|-------|----------|----------------|
| LIVE | Normal operation, all sources reporting | Full confidence |
| DEGRADED | Fewer than 3 source classes active | Rate published with warning flag |
| STALE | Rate older than 60 seconds | Consumers should use fallback or pause |
| HALTED | No valid rate computable | All dependent contracts should pause |

---

## Source Data Problem

**For initial launch (Phase 1):** Daeji runs its own EVM. There are no Aave, Compound,
or dYdX deployments on this chain. The ISFR sources don't exist natively.

### Solutions

**Option A: Cross-chain oracle feeds**
Validators run light clients or oracle bridges to observe rates on Ethereum mainnet,
Arbitrum, etc. Rates are reported to daeji via validator observations.

**Option B: Native DeFi bootstrapping**
Deploy lending protocols, structured vaults, and perp markets on daeji first.
ISFR derives from native sources. Chicken-and-egg problem.

**Option C: Hybrid (recommended)**
1. Phase 1: Hardcoded seed rate (e.g., 3.25%) published by a trusted operator
2. Phase 2: Cross-chain oracle feeds from external DeFi protocols
3. Phase 3: Mix of external feeds + native daeji DeFi sources
4. Phase 4: Fully native — daeji DeFi ecosystem is deep enough to self-source

### Phase 1 Implementation

```rust
struct ISFROracle {
    // Phase 1: single operator publishes rates via governance transaction
    rate: AtomicU64,
    block_height: AtomicU64,
    state: AtomicU8, // CircuitBreakerState
    history: RingBuffer<ISFRSnapshot, 8640>,
}
```

This is simple but functional. Consumers can build against the precompile interface
now. The aggregation logic upgrades behind the same interface later.

---

## Yield Perpetual Markets

The ISFR spec envisions a derivative market built on top of the rate:

### Contracts

| Contract | Purpose |
|----------|---------|
| `ClearingHouse` | Central matching engine for ISFR perpetual positions |
| `ClearingProfile` | Per-trader margin accounts, PnL tracking |
| `InsuranceFund` | Backstop for liquidation shortfalls |
| `LiquidationEngine` | Automatic position closure when margin < maintenance |

### Perpetual Mechanics

```
Long ISFR perp: profit when ISFR rises (rates going up)
Short ISFR perp: profit when ISFR falls (rates going down)

Funding = (markPrice - ISFR) × positionSize × (timeElapsed / 8h)
  → Longs pay shorts when mark > ISFR
  → Shorts pay longs when mark < ISFR
  → Converges mark to ISFR over time
```

### Contract Dependencies

```
ClearingHouse
  → reads ISFR from precompile (0xA01)
  → manages ClearingProfiles
  → calls LiquidationEngine when margin insufficient
  → Insurance Fund absorbs losses beyond trader margin

LiquidationEngine
  → reads current positions from ClearingProfile
  → reads ISFR for mark-to-market
  → closes positions via ClearingHouse
  → transfers excess to InsuranceFund
```

### Implementation Priority

The perpetual market is a Phase 3+ feature. The precompile and rate publication
come first. The contracts can be developed in parallel but don't need to deploy
until the rate has sufficient history and credibility.

---

## Implementation in daeji

### Precompile Registration

```rust
// In executor precompile registry
struct ISFRPrecompile {
    oracle: Arc<ISFROracle>,
}

impl Precompile for ISFRPrecompile {
    fn run(&self, input: &Bytes, gas_limit: u64) -> PrecompileResult {
        let selector = input[0];
        match selector {
            0x01 => self.current(gas_limit),
            0x02 => self.current_rate(gas_limit),
            0x03 => self.at(&input[1..], gas_limit),
            0x04 => self.twap(&input[1..], gas_limit),
            // ...
        }
    }
}
```

### Consensus Integration

The ISFR observation must be included in block validation. Validators need to:

1. Run source data collectors (Phase 2+)
2. Include observations in block proposals
3. Validate peer observations within tolerance
4. Aggregate at publication intervals

This touches `consensus/src/application.rs` (proposal building) and
`consensus/src/proposal.rs` (block construction).

### Storage

ISFR state lives in the precompile's reserved storage space, not in a contract's
storage. This makes it available at fixed gas cost regardless of state trie depth.

```
Precompile storage layout:
  [0x00] = current_rate (uint64)
  [0x01] = current_block (uint64)
  [0x02] = circuit_breaker_state (uint8)
  [0x03] = class_rates (4 × uint64 packed)
  [0x04..0x04+8640] = history ring buffer
```

---

## Testing Strategy

1. **Unit tests:** Rate aggregation math (weighted medians, outlier filtering)
2. **Circuit breaker tests:** State transitions under source failure scenarios
3. **Precompile tests:** Gas metering, ABI encoding, TWAP calculation accuracy
4. **Consensus tests:** Validators with different source observations converge on same rate
5. **Integration tests:** ClearingHouse reading ISFR precompile for mark-to-market
6. **Stress tests:** Rate publication under high block production (400ms blocks)
