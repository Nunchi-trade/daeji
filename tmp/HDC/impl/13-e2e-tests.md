# 13 -- End-to-End Tests for HDC

> **Status: EXISTS BUT HAS CRITICAL BLOCKERS**
>
> E2E test infrastructure exists at `crates/e2e/src/tests/hdc.rs` with 6 tests
> (bind, hamming, bundle, determinism, fast-blocks, mixed-ops). Basic structure
> is sound: `TestHarness`, `TestConfig`, transaction signing, genesis allocation.
> However, the tests have **5 critical blockers** that prevent them from actually
> validating HDC correctness end-to-end.
>
> **What works (DONE):**
> - Basic E2E test file structure with `mod hdc` registration
> - `call_hdc_precompile()`, `call_hamming()`, `call_bind()`, `call_bundle()` helpers
> - Transaction signing via `Evm::sign_eip1559_call()`
> - Genesis allocation and bootstrap tx submission
> - Consensus agreement verification (state root match across validators)
> - 14 precompile unit tests in `crates/hdc/chain/src/precompile.rs` (all pass)
>
> **CRITICAL BLOCKERS:**
>
> | ID | Blocker | Impact | File |
> |----|---------|--------|------|
> | AP01 | Tests don't verify precompile OUTPUT (only count finalized blocks) | Tests pass even if precompile silently reverts or returns wrong values | `crates/e2e/src/tests/hdc.rs` lines 94-185 |
> | AP02 | HDC precompile not registered in `TestApplication` | E2E harness may not include custom precompile in REVM execution context; tests may pass because the CALL to 0x09 silently fails | `crates/e2e/src/harness.rs`, `crates/node/runner/src/runner.rs` |
> | AP03 | Solidity `HdcLib` vs Rust precompile opcode table is NOW ALIGNED (both use 0x01-0x06 raw dispatch) but address may diverge if PR #42 moves to 0xA0C | Contracts compiled against wrong address will fail at runtime | `contracts/src/HdcPrecompile.sol`, `crates/hdc/chain/src/precompile.rs` |
> | AP04 | No mechanism to record/verify stored vectors (no recorder contract pattern) | Cannot assert that precompile computed the correct Hamming distance, bind result, etc. in a multi-validator E2E test | `crates/e2e/src/tests/hdc.rs` |
> | AP06 | `test_hdc_fast_blocks` doesn't actually test 50ms blocks (harness uses 1s leader_timeout) | Test name is misleading; 50ms block time is untested | `crates/e2e/src/tests/hdc.rs` lines 241-258 |
>
> **PRIORITY FIX CHECKLIST:**
>
> - [ ] **P1:** Register HDC precompile in TestApplication (add `.with_hdc_precompile()` to executor construction in harness)
> - [ ] **P1:** Reconcile precompile address: confirm 0x09 is canonical or migrate to 0xA0C per PR #42
> - [ ] **P1:** Add recorder contract pattern for output verification (see concrete code below)
> - [ ] **P2:** Add HNSW recall test (10k vectors, recall >= 0.95) -- see F09
> - [ ] **P2:** Add concurrent stress test (10k inserts, 15 tasks) -- see F10
> - [ ] **P2:** Add InsightBoard lifecycle E2E -- see F07
> - [ ] **P3:** Add benchmark `bench_hnsw_search_10000` (target: < 1ms median on 10k)
> - [ ] **P3:** Add `TestConfig::with_block_time_ms()` and thread through to simplex::Config
> - [ ] **P3:** Add `#[ignore]` to all non-ignored E2E tests (4 of 6 are missing it) -- see F12

> **Audience:** An agent (human or LLM) with no prior exposure to this codebase.
> This document tells you exactly what tests to write, where to put them, which
> APIs to use, and what mistakes to avoid.

---

## 1. Repository Orientation

| Path | What It Is |
|---|---|
| `/Users/will/dev/nunchi/daeji/` | Repo root |
| `crates/e2e/` | E2E test crate (`kora-e2e`) |
| `crates/e2e/src/harness.rs` | `TestHarness` -- runs multi-validator simulations |
| `crates/e2e/src/setup.rs` | `TestConfig`, `TestSetup` -- config and genesis helpers |
| `crates/e2e/src/node.rs` | `TestNode` -- handle for querying a running validator |
| `crates/e2e/src/tests/` | Existing test modules (`consensus.rs`, `execution.rs`, `resilience.rs`) |
| `crates/hdc/core/` | `kora-hdc` -- HDC algebra, search, knowledge (no chain deps) |
| `crates/hdc/chain/` | `kora-hdc-chain` -- precompile at `0x09`, InsightBoard, on-chain wiring |
| `crates/node/domain/src/evm.rs` | `Evm` helper -- `sign_eip1559_transfer`, `address_from_key` |
| `crates/node/executor/src/revm.rs` | `RevmExecutor` -- REVM-based block executor |

### Crates You Will Touch

| Crate | Test Type | Location |
|---|---|---|
| `kora-hdc` | Unit tests, integration tests | `crates/hdc/core/src/**/*.rs` (inline `#[cfg(test)]` modules) |
| `kora-hdc` | Integration tests (multi-module) | `crates/hdc/core/tests/*.rs` |
| `kora-e2e` | E2E tests | `crates/e2e/src/tests/hdc.rs` |
| `kora-hdc` | Benchmarks | `crates/hdc/core/benches/*.rs` |

---

## 2. Test Infrastructure

### 2.1 `TestConfig`

Configures the simulated network. Defined in `crates/e2e/src/setup.rs`.

```rust
use crate::{TestConfig, TestHarness, TestSetup};

let config = TestConfig::default()    // 4 validators, threshold 3, seed 42
    .with_validators(4)               // set validator count (auto-sets BFT threshold)
    .with_max_blocks(5)               // stop after N finalized blocks
    .with_seed(42)                    // deterministic PRNG seed
    .with_timeout(Duration::from_secs(30))
    .with_link(SimLinkConfig {        // network simulation
        latency: Duration::from_millis(10),
        jitter: Duration::from_millis(1),
        success_rate: 1.0,
    });
```

Defaults: `validators=4`, `threshold=3`, `seed=42`, `chain_id=1337`,
`gas_limit=30_000_000`, `max_blocks=5`, `timeout=30s`.

### 2.2 `TestSetup`

Genesis allocations and bootstrap transactions. Defined in `crates/e2e/src/setup.rs`.

```rust
// Empty genesis (no accounts, no txs)
let setup = TestSetup::empty();

// Pre-built: single transfer from sender to receiver
let setup = TestSetup::simple_transfer(config.chain_id);

// Pre-built: N independent transfers
let setup = TestSetup::multi_transfer(config.chain_id, 5);

// Pre-built: N txs from same sender with sequential nonces
let setup = TestSetup::sequential_nonces(config.chain_id, 3);
```

Fields you set directly for custom scenarios:

```rust
let setup = TestSetup {
    genesis_alloc: vec![(address, balance), ...],  // funded at genesis
    bootstrap_txs: vec![tx1, tx2, ...],            // submitted to mempool before block 1
    expected_balances: vec![(address, balance), ...], // verified after finalization
};
```

### 2.3 `TestHarness`

Runs the test. Blocks until all validators finalize `max_blocks` or `timeout` fires.

```rust
let outcome = TestHarness::run(config, setup).expect("test should pass");
assert_eq!(outcome.blocks_finalized, 5);
assert_eq!(outcome.state_root, expected_root); // all validators agree
assert_ne!(outcome.seed, B256::ZERO);          // threshold sig worked
```

The harness:
1. Generates threshold BLS signing schemes from the seed
2. Creates a simulated network via `kora-transport-sim` (no real TCP)
3. Starts N validator nodes, each with its own QMDB partition
4. Submits bootstrap transactions to each node's mempool
5. Waits for all nodes to finalize `max_blocks` blocks
6. Verifies all nodes agree on state root, seed, and expected balances
7. Returns `TestOutcome` with `finalized_head`, `state_root`, `seed`, counts

### 2.4 `SimLinkConfig`

Controls the simulated network between validators.

```rust
use kora_transport_sim::SimLinkConfig;

SimLinkConfig {
    latency: Duration::from_millis(10),  // one-way latency
    jitter: Duration::from_millis(1),    // +/- jitter on latency
    success_rate: 1.0,                   // 1.0 = no drops, 0.9 = 10% loss
}
```

### 2.5 `TestNode`

Handle returned by the harness for post-execution queries. Available on nodes
via `outcome` indirectly, or in custom harness extensions.

```rust
node.submit_tx(tx).await;                          // submit to mempool
node.query_balance(digest, address).await;          // balance at block
node.query_state_root(digest).await;                // state root at block
node.query_seed(digest).await;                      // prevrandao at block
```

### 2.6 Run Command

```bash
# E2E tests MUST run single-threaded (QMDB file conflicts otherwise)
cargo test -p kora-e2e --test-threads=1

# Unit/integration tests in kora-hdc can run in parallel
cargo test -p kora-hdc

# Benchmarks
cargo bench -p kora-hdc
```

---

## 3. Unit Tests (in `kora-hdc` crate)

These go inside `#[cfg(test)] mod tests { ... }` blocks in the relevant source
files. They test the HDC algebra in isolation -- no chain, no network, no QMDB.

### File: `crates/hdc/core/src/algebra.rs`

#### 3.1 Vector Operations -- bind is self-inverse

```rust
#[test]
fn bind_is_self_inverse() {
    let mut rng = StdRng::seed_from_u64(42);
    let a = HdcVector::random(&mut rng);
    let b = HdcVector::random(&mut rng);

    let bound = a.bind(&b);
    let recovered = bound.bind(&b);

    // bind(bind(a, b), b) == a for XOR-based bind
    assert_eq!(a, recovered);
}
```

#### 3.2 Bundle majority vote

```rust
#[test]
fn bundle_majority_vote() {
    let mut rng = StdRng::seed_from_u64(42);
    let a = HdcVector::random(&mut rng);
    let b = HdcVector::random(&mut rng);
    let c = HdcVector::random(&mut rng);

    // Bundle of {a, a, b} should be closer to a than to b or c
    let bundle = HdcVector::bundle(&[a.clone(), a.clone(), b.clone()]);

    assert!(bundle.hamming_distance(&a) < bundle.hamming_distance(&b));
    assert!(bundle.hamming_distance(&a) < bundle.hamming_distance(&c));
}
```

#### 3.3 Permute rotation

```rust
#[test]
fn permute_rotation() {
    let mut rng = StdRng::seed_from_u64(42);
    let v = HdcVector::random(&mut rng);

    // Permuting by D should return to the original
    let d = HdcVector::DIMENSIONS;
    let mut rotated = v.clone();
    for _ in 0..d {
        rotated = rotated.permute(1);
    }
    assert_eq!(v, rotated);
}
```

#### 3.4 Determinism -- same seed produces same vector

```rust
#[test]
fn determinism_same_seed_same_vector() {
    let v1 = HdcVector::random(&mut StdRng::seed_from_u64(999));
    let v2 = HdcVector::random(&mut StdRng::seed_from_u64(999));
    assert_eq!(v1, v2);
}

#[test]
fn determinism_different_seed_different_vector() {
    let v1 = HdcVector::random(&mut StdRng::seed_from_u64(1));
    let v2 = HdcVector::random(&mut StdRng::seed_from_u64(2));
    assert_ne!(v1, v2);
}
```

#### 3.5 Serialization round-trip

```rust
#[test]
fn serialize_deserialize_roundtrip() {
    let mut rng = StdRng::seed_from_u64(42);
    let original = HdcVector::random(&mut rng);

    let bytes = original.to_bytes();
    let restored = HdcVector::from_bytes(&bytes).expect("valid deserialization");

    assert_eq!(original, restored);
}
```

#### 3.6 Hamming distance -- known vectors, known distances

```rust
#[test]
fn hamming_distance_identical_is_zero() {
    let mut rng = StdRng::seed_from_u64(42);
    let v = HdcVector::random(&mut rng);
    assert_eq!(v.hamming_distance(&v), 0);
}

#[test]
fn hamming_distance_complement_is_max() {
    let mut rng = StdRng::seed_from_u64(42);
    let v = HdcVector::random(&mut rng);
    let complement = v.complement();
    assert_eq!(v.hamming_distance(&complement), HdcVector::DIMENSIONS);
}

#[test]
fn hamming_distance_random_vectors_near_half() {
    // Random vectors concentrate around D/2 = 5120
    let mut rng = StdRng::seed_from_u64(42);
    let a = HdcVector::random(&mut rng);
    let b = HdcVector::random(&mut rng);
    let d = a.hamming_distance(&b);
    let half = HdcVector::DIMENSIONS / 2;
    let sigma = (HdcVector::DIMENSIONS as f64).sqrt() as usize / 2;
    // Within 6 sigma of D/2
    assert!((d as isize - half as isize).unsigned_abs() < 6 * sigma);
}
```

#### 3.7 Bundle capacity -- 50 vectors retrievable

```rust
#[test]
fn bundle_capacity_50_vectors() {
    let mut rng = StdRng::seed_from_u64(42);
    let vectors: Vec<HdcVector> = (0..50).map(|_| HdcVector::random(&mut rng)).collect();

    let bundle = HdcVector::bundle(&vectors);

    // Each constituent should have similarity > 0.526 with the bundle
    // (Hamming distance < 4854 out of 10240)
    let threshold = (HdcVector::DIMENSIONS as f64 * 0.474).ceil() as usize; // 4854

    for (i, v) in vectors.iter().enumerate() {
        let dist = bundle.hamming_distance(v);
        assert!(
            dist < threshold,
            "vector {i} has hamming distance {dist} >= threshold {threshold}"
        );
    }
}
```

#### 3.8 Anti-knowledge

```rust
#[test]
fn anti_knowledge_double_negation() {
    let mut rng = StdRng::seed_from_u64(42);
    let x = HdcVector::random(&mut rng);
    let anti_x = x.anti();
    let anti_anti_x = anti_x.anti();

    // anti(anti(x)) == x
    assert_eq!(x, anti_anti_x);
}

#[test]
fn anti_knowledge_orthogonal() {
    let mut rng = StdRng::seed_from_u64(42);
    let x = HdcVector::random(&mut rng);
    let anti_x = x.anti();

    // anti(x) should be approximately orthogonal to x
    // i.e., hamming distance ~= D/2
    let dist = x.hamming_distance(&anti_x);
    let half = HdcVector::DIMENSIONS / 2;
    let tolerance = HdcVector::DIMENSIONS / 20; // 5% tolerance
    assert!(
        (dist as isize - half as isize).unsigned_abs() < tolerance,
        "expected ~{half}, got {dist}"
    );
}
```

---

## 4. Integration Tests (in `kora-hdc` crate)

These test multiple modules working together. Place them in `crates/hdc/core/tests/`.

### File: `crates/hdc/core/tests/search.rs`

#### 4.1 BruteForceIndex -- insert and search

```rust
use kora_hdc::{HdcVector, BruteForceIndex};
use rand::SeedableRng;
use rand::rngs::StdRng;

#[test]
fn brute_force_insert_1000_search_top_k() {
    let mut rng = StdRng::seed_from_u64(42);
    let mut index = BruteForceIndex::new();

    // Insert 1000 random vectors
    let vectors: Vec<HdcVector> = (0..1000)
        .map(|_| HdcVector::random(&mut rng))
        .collect();

    for (i, v) in vectors.iter().enumerate() {
        index.insert(i as u64, v.clone());
    }

    // Search for each of the first 10 vectors; they should be their own top-1
    for i in 0..10 {
        let results = index.search(&vectors[i], 5);
        assert_eq!(
            results[0].0, i as u64,
            "top-1 for vector {i} should be itself"
        );
        assert_eq!(results[0].1, 0, "distance to self should be 0");
    }
}
```

#### 4.2 HNSW -- recall vs brute force

```rust
#[test]
fn hnsw_recall_vs_brute_force() {
    let mut rng = StdRng::seed_from_u64(42);

    let mut brute = BruteForceIndex::new();
    let mut hnsw = HnswIndex::new(/* M= */ 16, /* ef_construction= */ 200);

    let vectors: Vec<HdcVector> = (0..10_000)
        .map(|_| HdcVector::random(&mut rng))
        .collect();

    for (i, v) in vectors.iter().enumerate() {
        let id = i as u64;
        brute.insert(id, v.clone());
        hnsw.insert(id, v.clone());
    }

    // Sample 100 queries
    let queries: Vec<HdcVector> = (0..100)
        .map(|_| HdcVector::random(&mut rng))
        .collect();

    let k = 10;
    let mut hits = 0usize;
    let total = queries.len() * k;

    for q in &queries {
        let brute_results: Vec<u64> = brute.search(q, k).iter().map(|r| r.0).collect();
        let hnsw_results: Vec<u64> = hnsw.search(q, k).iter().map(|r| r.0).collect();

        for id in &hnsw_results {
            if brute_results.contains(id) {
                hits += 1;
            }
        }
    }

    let recall = hits as f64 / total as f64;
    assert!(
        recall > 0.95,
        "HNSW recall {recall:.3} is below 0.95 threshold"
    );
}
```

### File: `crates/hdc/core/tests/knowledge.rs`

#### 4.3 Knowledge decay

```rust
#[test]
fn knowledge_decay_reduces_balance() {
    let mut rng = StdRng::seed_from_u64(42);
    let mut store = KnowledgeStore::new();

    let vec = HdcVector::random(&mut rng);
    let entry_id = store.insert(KnowledgeEntry::new(
        vec,
        KnowledgeKind::Insight,
        /* initial_balance= */ 1.0,
    ));

    let balance_before = store.get(entry_id).unwrap().balance();

    // Tick forward (simulates time passing; lambda = 0.005/hour)
    // After 138.6 hours (one half-life), balance should be ~0.5
    store.tick(138.6 * 3600.0); // seconds

    let balance_after = store.get(entry_id).unwrap().balance();
    assert!(
        balance_after < balance_before,
        "balance should decrease: {balance_before} -> {balance_after}"
    );
    // Within 5% of half
    assert!(
        (balance_after - 0.5).abs() < 0.05,
        "after one half-life, balance should be ~0.5, got {balance_after}"
    );
}
```

#### 4.4 Tier promotion

```rust
#[test]
fn tier_promotion_transient_to_working_to_consolidated() {
    let mut rng = StdRng::seed_from_u64(42);
    let mut store = KnowledgeStore::new();

    let vec = HdcVector::random(&mut rng);
    let entry_id = store.insert(KnowledgeEntry::new(
        vec,
        KnowledgeKind::Insight,
        1.0,
    ));

    // Initially: Transient tier
    assert_eq!(store.get(entry_id).unwrap().tier(), RetentionTier::Transient);

    // Confirm 3 times -> Working
    for _ in 0..3 {
        store.reinforce(entry_id);
    }
    assert_eq!(store.get(entry_id).unwrap().tier(), RetentionTier::Working);

    // Confirm 10 total times -> Consolidated
    for _ in 0..7 {
        store.reinforce(entry_id);
    }
    assert_eq!(store.get(entry_id).unwrap().tier(), RetentionTier::Consolidated);
}
```

---

## 5. E2E Tests (in `kora-e2e` crate)

These test the full stack: HDC operations executing inside the EVM on a
multi-validator simulated network with consensus.

### File: `crates/e2e/src/tests/hdc.rs`

Register this module in `crates/e2e/src/tests/mod.rs`:

```rust
// crates/e2e/src/tests/mod.rs
mod consensus;
mod execution;
mod hdc;        // <-- add this line
mod resilience;
```

Add `kora-hdc-chain` as a dependency of `kora-e2e` in `crates/e2e/Cargo.toml`:

```toml
[dependencies]
# ... existing deps ...
kora-hdc-chain.workspace = true
```

### 5.1 How to Extend `TestSetup` for HDC Genesis State

The HDC precompile is registered at address `0x09`. It does not need genesis
storage -- it is a native REVM precompile, not a deployed contract. The
InsightBoard and PheromoneRegistry are Solidity contracts that DO need to be
deployed.

To deploy contracts in tests, include contract-creation transactions in
`bootstrap_txs`:

```rust
impl TestSetup {
    /// Create a setup that deploys the InsightBoard contract.
    pub fn with_insight_board(chain_id: u64) -> Self {
        let deployer_key = SigningKey::from_bytes(&[0xAA; 32].into())
            .expect("valid key");
        let deployer = Evm::address_from_key(&deployer_key);

        // Fund the deployer
        let initial_balance = U256::from(10_000_000_000u64);

        // Create the deployment transaction
        // The bytecode comes from compiling InsightBoard.sol
        let deploy_tx = Evm::sign_eip1559_create(
            &deployer_key,
            chain_id,
            INSIGHT_BOARD_BYTECODE.into(),
            /* nonce= */ 0,
            /* gas_limit= */ 5_000_000,
        );

        Self {
            genesis_alloc: vec![(deployer, initial_balance)],
            bootstrap_txs: vec![deploy_tx],
            expected_balances: vec![],
        }
    }
}
```

You will need to add a `sign_eip1559_create` helper to `Evm` (in
`crates/node/domain/src/evm.rs`). It is identical to `sign_eip1559_transfer`
except it uses `TxKind::Create` instead of `TxKind::Call(to)` and puts the
bytecode in `input`:

```rust
impl Evm {
    pub fn sign_eip1559_create(
        key: &SigningKey,
        chain_id: u64,
        bytecode: Bytes,
        nonce: u64,
        gas_limit: u64,
    ) -> Tx {
        let tx = TxEip1559 {
            chain_id,
            nonce,
            gas_limit,
            max_fee_per_gas: 0,
            max_priority_fee_per_gas: 0,
            to: TxKind::Create,
            value: U256::ZERO,
            access_list: Default::default(),
            input: bytecode,
        };
        // ... same signing logic as sign_eip1559_transfer ...
    }
}
```

### 5.2 How to Call Precompiles from Test Transactions

The HDC precompile lives at address `0x09`. To call it, send a transaction with
`to = 0x09` and the precompile's packed raw-opcode input in the `input` field.
Do not use a 4-byte Solidity function selector unless the Rust precompile is
explicitly changed to a Solidity-ABI dispatcher. The current Rust dispatcher
reads `input[0]` as the opcode.

```rust
fn call_hamming_precompile(
    key: &SigningKey,
    chain_id: u64,
    vector_a: &[u8],  // 1280 bytes (10240 bits)
    vector_b: &[u8],  // 1280 bytes
    nonce: u64,
) -> Tx {
    // Packed precompile ABI: opcode(0x01) + vector_a(1280 bytes) + vector_b(1280 bytes)
    let mut input = Vec::with_capacity(1 + 1280 + 1280);
    input.push(0x01);
    input.extend_from_slice(vector_a);
    input.extend_from_slice(vector_b);

    let precompile_address = Address::from_slice(&{
        let mut addr = [0u8; 20];
        addr[19] = 0x09;
        addr
    });

    let tx = TxEip1559 {
        chain_id,
        nonce,
        gas_limit: 100_000,  // precompile gas is cheap
        max_fee_per_gas: 0,
        max_priority_fee_per_gas: 0,
        to: TxKind::Call(precompile_address),
        value: U256::ZERO,
        access_list: Default::default(),
        input: Bytes::from(input),
    };
    // ... sign and encode as in sign_eip1559_transfer ...
}
```

### 5.3 How to Deploy Solidity Contracts in E2E Tests

Two approaches:

**Approach A -- Compile offline, embed bytecode.** Compile the Solidity contract
with `solc` or `forge build`, then include the hex-encoded bytecode as a `const`
in the test file. This is the simplest approach and avoids runtime compilation
dependencies.

```rust
// Compiled bytecode of InsightBoard.sol (output of `solc --bin`)
const INSIGHT_BOARD_BYTECODE: &[u8] = include_bytes!("../../contracts/InsightBoard.bin");

// Or as a hex constant:
const INSIGHT_BOARD_BYTECODE_HEX: &str = "608060405234801561001057...";
```

**Approach B -- Use forge artifacts.** If `forge build` runs as part of CI,
read the JSON artifacts at test time:

```rust
fn load_contract_bytecode(name: &str) -> Bytes {
    let artifact_path = format!("contracts/out/{name}.sol/{name}.json");
    let json: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(&artifact_path)
            .unwrap_or_else(|_| panic!("missing artifact: {artifact_path}"))
    ).unwrap();
    let hex = json["bytecode"]["object"].as_str().unwrap().trim_start_matches("0x");
    Bytes::from(hex::decode(hex).unwrap())
}
```

**Approach A is recommended** for determinism and CI simplicity.

### 5.4 Precompile Test

```rust
//! crates/e2e/src/tests/hdc.rs

use std::time::Duration;
use alloy_primitives::{Address, U256};
use crate::{TestConfig, TestHarness, TestSetup};

/// Test that the HDC Hamming distance precompile at 0x09 produces correct results.
///
/// Deploys a contract that calls the precompile, executes it across 3 validators,
/// and verifies all validators agree on the result.
#[test]
fn test_hdc_precompile_hamming_distance() {
    let config = TestConfig::default()
        .with_validators(4)
        .with_max_blocks(3)
        .with_timeout(Duration::from_secs(30));

    // Setup: fund a caller, include a tx that calls 0x09 with two known vectors
    let setup = TestSetup::hdc_precompile_hamming(config.chain_id);

    let outcome = TestHarness::run(config, setup)
        .expect("precompile call should succeed and reach consensus");

    assert_eq!(outcome.blocks_finalized, 3);
    // State convergence is verified internally by the harness.
    // If we got here, all 4 validators computed the same Hamming distance.
}
```

### 5.5 InsightBoard Test

```rust
/// Test the InsightBoard contract lifecycle: submit -> confirm -> state transitions.
#[test]
fn test_insight_board_submit_and_confirm() {
    let config = TestConfig::default()
        .with_validators(4)
        .with_max_blocks(5)
        .with_timeout(Duration::from_secs(45));

    // Setup: deploy InsightBoard, fund agents, submit + confirm an insight
    let setup = TestSetup::insight_board_lifecycle(config.chain_id);

    let outcome = TestHarness::run(config, setup)
        .expect("InsightBoard lifecycle should reach consensus");

    assert_eq!(outcome.blocks_finalized, 5);
}
```

### 5.6 PheromoneRegistry Test

```rust
/// Test PheromoneRegistry: deposit pheromone, verify decay over blocks.
#[test]
fn test_pheromone_registry_decay() {
    let config = TestConfig::default()
        .with_validators(4)
        .with_max_blocks(10)
        .with_timeout(Duration::from_secs(60));

    let setup = TestSetup::pheromone_registry_decay(config.chain_id);

    let outcome = TestHarness::run(config, setup)
        .expect("pheromone decay should reach consensus");

    assert_eq!(outcome.blocks_finalized, 10);
}
```

### 5.7 Consensus Determinism after HDC Operations

```rust
/// Verify that all validators agree on state root after HDC-heavy transactions.
#[test]
fn test_consensus_determinism_with_hdc() {
    let config = TestConfig::default()
        .with_validators(4)
        .with_max_blocks(5)
        .with_seed(42)
        .with_timeout(Duration::from_secs(45));

    let setup = TestSetup::hdc_determinism_scenario(config.chain_id);

    // Run twice with the same seed
    let outcome1 = TestHarness::run(config.clone(), setup.clone())
        .expect("first run");
    let outcome2 = TestHarness::run(config, setup)
        .expect("second run");

    // Same seed + same txs = same state root
    assert_eq!(outcome1.state_root, outcome2.state_root);
}
```

### 5.8 50ms Block Time

```rust
/// Verify that blocks are produced at 50ms intervals.
#[test]
fn test_50ms_block_time() {
    let config = TestConfig::default()
        .with_validators(4)
        .with_max_blocks(20)
        .with_link(SimLinkConfig {
            latency: Duration::from_millis(5),   // keep latency well below block time
            jitter: Duration::from_millis(1),
            success_rate: 1.0,
        })
        .with_timeout(Duration::from_secs(30));

    // NOTE: the block time itself is controlled by the leader_timeout and
    // certification_timeout in the simplex::Config inside harness.rs.
    // To test 50ms blocks, you must add a .with_block_time_ms(50) builder
    // method on TestConfig and thread it through to the engine config.

    let setup = TestSetup::empty();

    let outcome = TestHarness::run(config, setup)
        .expect("50ms blocks should finalize");

    assert_eq!(outcome.blocks_finalized, 20);
}
```

---

## 6. Stress Tests

### File: `crates/hdc/core/tests/stress.rs`

#### 6.1 High-Volume Inserts

```rust
#[test]
fn stress_10k_knowledge_entries() {
    let mut rng = StdRng::seed_from_u64(42);
    let mut store = KnowledgeStore::new();

    // Insert 10,000 entries
    let vectors: Vec<HdcVector> = (0..10_000)
        .map(|_| HdcVector::random(&mut rng))
        .collect();

    for (i, v) in vectors.iter().enumerate() {
        store.insert(KnowledgeEntry::new(
            v.clone(),
            KnowledgeKind::Insight,
            1.0,
        ));
    }

    // Search should still work correctly
    let query = &vectors[5000];
    let results = store.search(query, 10);

    // The query vector itself should be the top result
    assert_eq!(results[0].distance, 0);
}
```

#### 6.2 Concurrent Operations

```rust
#[tokio::test]
async fn stress_concurrent_inserts_and_searches() {
    use std::sync::Arc;
    use tokio::task;

    let store = Arc::new(tokio::sync::RwLock::new(KnowledgeStore::new()));

    // Spawn 10 inserter tasks
    let mut handles = Vec::new();
    for t in 0..10 {
        let store = store.clone();
        handles.push(task::spawn(async move {
            let mut rng = StdRng::seed_from_u64(t);
            for _ in 0..1000 {
                let v = HdcVector::random(&mut rng);
                let mut s = store.write().await;
                s.insert(KnowledgeEntry::new(v, KnowledgeKind::Insight, 1.0));
            }
        }));
    }

    // Spawn 5 searcher tasks concurrently
    for t in 0..5 {
        let store = store.clone();
        handles.push(task::spawn(async move {
            let mut rng = StdRng::seed_from_u64(100 + t);
            for _ in 0..100 {
                let q = HdcVector::random(&mut rng);
                let s = store.read().await;
                let _ = s.search(&q, 10);
            }
        }));
    }

    for h in handles {
        h.await.unwrap();
    }

    // Verify final state
    let s = store.read().await;
    assert_eq!(s.len(), 10_000); // 10 tasks * 1000 inserts
}
```

---

## 7. Performance Benchmarks

### File: `crates/hdc/core/benches/hdc_bench.rs`

Add to `crates/hdc/core/Cargo.toml`:

```toml
[dev-dependencies]
criterion = { version = "0.5", features = ["html_reports"] }
rand = "0.8"

[[bench]]
name = "hdc_bench"
harness = false
```

```rust
use criterion::{Criterion, black_box, criterion_group, criterion_main};
use kora_hdc::HdcVector;
use rand::SeedableRng;
use rand::rngs::StdRng;

fn bench_hamming_distance(c: &mut Criterion) {
    let mut rng = StdRng::seed_from_u64(42);
    let a = HdcVector::random(&mut rng);
    let b = HdcVector::random(&mut rng);

    c.bench_function("hamming_distance", |bencher| {
        bencher.iter(|| black_box(a.hamming_distance(&b)))
    });
}

fn bench_bind(c: &mut Criterion) {
    let mut rng = StdRng::seed_from_u64(42);
    let a = HdcVector::random(&mut rng);
    let b = HdcVector::random(&mut rng);

    c.bench_function("bind", |bencher| {
        bencher.iter(|| black_box(a.bind(&b)))
    });
}

fn bench_bundle_50(c: &mut Criterion) {
    let mut rng = StdRng::seed_from_u64(42);
    let vecs: Vec<HdcVector> = (0..50).map(|_| HdcVector::random(&mut rng)).collect();

    c.bench_function("bundle_50", |bencher| {
        bencher.iter(|| black_box(HdcVector::bundle(&vecs)))
    });
}

fn bench_brute_force_search_1000(c: &mut Criterion) {
    let mut rng = StdRng::seed_from_u64(42);
    let mut index = kora_hdc::BruteForceIndex::new();
    for i in 0..1000 {
        index.insert(i, HdcVector::random(&mut rng));
    }
    let query = HdcVector::random(&mut rng);

    c.bench_function("brute_force_search_1000", |bencher| {
        bencher.iter(|| black_box(index.search(&query, 10)))
    });
}

fn bench_hnsw_search_10000(c: &mut Criterion) {
    let mut rng = StdRng::seed_from_u64(42);
    let mut index = kora_hdc::HnswIndex::new(16, 200);
    for i in 0..10_000 {
        index.insert(i, HdcVector::random(&mut rng));
    }
    let query = HdcVector::random(&mut rng);

    c.bench_function("hnsw_search_10000", |bencher| {
        bencher.iter(|| black_box(index.search(&query, 10)))
    });
}

fn bench_serialize_roundtrip(c: &mut Criterion) {
    let mut rng = StdRng::seed_from_u64(42);
    let v = HdcVector::random(&mut rng);

    c.bench_function("serialize_roundtrip", |bencher| {
        bencher.iter(|| {
            let bytes = v.to_bytes();
            black_box(HdcVector::from_bytes(&bytes).unwrap())
        })
    });
}

criterion_group!(
    benches,
    bench_hamming_distance,
    bench_bind,
    bench_bundle_50,
    bench_brute_force_search_1000,
    bench_hnsw_search_10000,
    bench_serialize_roundtrip,
);
criterion_main!(benches);
```

### Expected Performance Targets

| Operation | Target | Notes |
|---|---|---|
| Hamming distance | < 50 ns | SIMD hot path (AVX2/NEON); < 10 ns with AVX-512 VPOPCNTDQ |
| Bind (XOR) | < 30 ns | Pure bitwise, no allocation |
| Bundle (50 vectors) | < 5 us | Majority vote over 50 * 1280 bytes |
| Brute-force search (1K) | < 100 us | 1000 Hamming distances + sort |
| HNSW search (10K) | < 1 ms | Approximate, depends on graph connectivity |
| Serialize round-trip | < 500 ns | 1280 bytes memcpy |

---

## 8. Anti-Patterns

These are common mistakes. Do not make them.

### 8.1 Do NOT run E2E tests in parallel

```bash
# WRONG -- QMDB partitions will conflict, causing random failures
cargo test -p kora-e2e

# CORRECT
cargo test -p kora-e2e -- --test-threads=1
```

QMDB uses file-based storage with partitions named by a prefix. Even though the
harness generates unique partition names (using timestamp + counter + seed), the
underlying file system operations are not safe under concurrent test execution.
Always use `--test-threads=1` for `kora-e2e`.

Unit and integration tests in `kora-hdc` do NOT use QMDB and CAN run in parallel.

### 8.2 Do NOT hardcode block numbers

```rust
// WRONG -- fragile, breaks if test config changes
assert_eq!(some_value_at_block_5, expected);

// CORRECT -- use relative references
let outcome = TestHarness::run(config, setup).unwrap();
// outcome.finalized_head is the last block's digest
// Use it to query state
```

Block numbers are a function of `config.max_blocks`. Always reference state via
the `ConsensusDigest` returned in `TestOutcome::finalized_head`, not absolute
block heights.

### 8.3 Do NOT forget determinism checks

Every HDC operation that will execute on-chain MUST be deterministic. This means:

- **No floating-point** in consensus-path code. Use fixed-point or integer arithmetic.
- **No `HashMap` iteration order.** Use `BTreeMap` for deterministic ordering.
- **No unseeded RNG.** All randomness must come from seeded PRNGs.
- **No platform-dependent SIMD semantics.** SIMD is fine for performance, but the
  scalar fallback must produce identical results. Test both paths.

Write a determinism test for every new on-chain HDC operation:

```rust
#[test]
fn operation_is_deterministic() {
    for seed in [1, 42, 999, u64::MAX] {
        let result1 = run_operation(seed);
        let result2 = run_operation(seed);
        assert_eq!(result1, result2, "non-deterministic with seed {seed}");
    }
}
```

### 8.4 Do NOT test HNSW recall with too few vectors

HNSW needs meaningful graph density to work correctly. Testing with fewer than
1,000 vectors gives misleading recall numbers. Use at least 10,000 for recall
benchmarks.

### 8.5 Do NOT skip the `#[ignore]` annotation for slow tests

Follow the existing convention in `crates/e2e/src/tests/`:

```rust
#[test]
#[ignore = "flaky when run in parallel - run with --test-threads=1"]
fn test_something_slow() { ... }
```

This lets `cargo test` skip slow tests by default. Run them explicitly with
`cargo test -- --ignored` or `cargo test -- --include-ignored`.

### 8.6 Do NOT use `assert!(x == y)` for diagnostics

```rust
// WRONG -- failure message is just "assertion failed"
assert!(outcome.state_root == expected_root);

// CORRECT -- failure message shows both values
assert_eq!(outcome.state_root, expected_root);
```

### 8.7 Do NOT allocate in the Hamming distance hot path

The Hamming distance function (`hamming_distance`) must not allocate. No `Vec`,
no `String`, no `Box`. It operates directly on `[u64; 160]` using XOR + popcount.
Benchmarks will catch regressions, but do not introduce allocations in the first
place.

---

## 9. Test File Summary

| File | Type | What It Tests |
|---|---|---|
| `crates/hdc/core/src/algebra.rs` (inline) | Unit | bind self-inverse, bundle majority, permute rotation, determinism, serialization, hamming distance, bundle capacity, anti-knowledge |
| `crates/hdc/core/tests/search.rs` | Integration | BruteForceIndex (1K), HNSW recall vs brute force (10K) |
| `crates/hdc/core/tests/knowledge.rs` | Integration | Knowledge decay, tier promotion |
| `crates/hdc/core/tests/stress.rs` | Stress | 10K inserts, concurrent inserts + searches |
| `crates/hdc/core/benches/hdc_bench.rs` | Benchmark | Hamming, bind, bundle, brute-force search, HNSW search, serialization |
| `crates/e2e/src/tests/hdc.rs` | E2E | Precompile call, InsightBoard lifecycle, PheromoneRegistry decay, consensus determinism, 50ms block time |

---

## 10. Checklist

Use this to track progress. Each item maps to a test described above.

### Unit Tests (kora-hdc)

- [ ] `bind_is_self_inverse`
- [ ] `bundle_majority_vote`
- [ ] `permute_rotation`
- [ ] `determinism_same_seed_same_vector`
- [ ] `determinism_different_seed_different_vector`
- [ ] `serialize_deserialize_roundtrip`
- [ ] `hamming_distance_identical_is_zero`
- [ ] `hamming_distance_complement_is_max`
- [ ] `hamming_distance_random_vectors_near_half`
- [ ] `bundle_capacity_50_vectors`
- [ ] `anti_knowledge_double_negation`
- [ ] `anti_knowledge_orthogonal`

### Integration Tests (kora-hdc)

- [ ] `brute_force_insert_1000_search_top_k`
- [ ] `hnsw_recall_vs_brute_force` (recall > 0.95)
- [ ] `knowledge_decay_reduces_balance`
- [ ] `tier_promotion_transient_to_working_to_consolidated`

### E2E Tests (kora-e2e)

- [ ] `test_hdc_precompile_hamming_distance`
- [ ] `test_insight_board_submit_and_confirm`
- [ ] `test_pheromone_registry_decay`
- [ ] `test_consensus_determinism_with_hdc`
- [ ] `test_50ms_block_time`

### Stress Tests

- [ ] `stress_10k_knowledge_entries`
- [ ] `stress_concurrent_inserts_and_searches`

### Benchmarks

- [ ] `bench_hamming_distance` (target: < 50 ns)
- [ ] `bench_bind` (target: < 30 ns)
- [ ] `bench_bundle_50` (target: < 5 us)
- [ ] `bench_brute_force_search_1000` (target: < 100 us)
- [ ] `bench_hnsw_search_10000` (target: < 1 ms)
- [ ] `bench_serialize_roundtrip` (target: < 500 ns)

### Infrastructure

- [ ] Add `hdc` module to `crates/e2e/src/tests/mod.rs`
- [ ] Add `kora-hdc-chain` dependency to `crates/e2e/Cargo.toml`
- [ ] Add `sign_eip1559_create` helper to `crates/node/domain/src/evm.rs`
- [ ] Add `criterion` dev-dependency and `[[bench]]` to `crates/hdc/core/Cargo.toml`
- [ ] Compile InsightBoard and PheromoneRegistry Solidity contracts, embed bytecode
- [ ] Verify all E2E tests pass with `--test-threads=1`
- [ ] Verify all benchmarks produce results within expected ranges

---

## 11. Recorder Contract Pattern for Output Verification

The fundamental gap in the current E2E tests is that precompile return values
are not observable after block finalization. The `TestHarness` only exposes
`expected_balances` and `state_root`. To verify precompile outputs, deploy a
**recorder contract** that calls the precompile and stores the result in
contract storage, then query the storage post-finalization.

### 11.1 Recorder Contract (Solidity)

```solidity
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.20;

/// @title HdcRecorder
/// @notice Calls the HDC precompile and records results in storage for E2E verification.
contract HdcRecorder {
    address constant HDC = address(0x09);

    // Recorded results
    uint32 public lastHammingDistance;
    bytes public lastBindResult;
    bytes public lastBundleResult;
    bool public lastIsSimilar;
    bytes32 public lastVectorId;

    function recordHamming(bytes calldata a, bytes calldata b) external {
        bytes memory payload = abi.encodePacked(uint8(0x01), a, b);
        (bool ok, bytes memory ret) = HDC.staticcall(payload);
        require(ok, "hamming failed");
        lastHammingDistance = abi.decode(ret, (uint32));
    }

    function recordBind(bytes calldata a, bytes calldata b) external {
        bytes memory payload = abi.encodePacked(uint8(0x02), a, b);
        (bool ok, bytes memory ret) = HDC.staticcall(payload);
        require(ok, "bind failed");
        lastBindResult = ret;
    }

    function recordBundle(bytes[] calldata vectors) external {
        bytes memory payload = abi.encodePacked(uint8(0x03), uint32(vectors.length));
        for (uint256 i = 0; i < vectors.length; i++) {
            payload = abi.encodePacked(payload, vectors[i]);
        }
        (bool ok, bytes memory ret) = HDC.staticcall(payload);
        require(ok, "bundle failed");
        lastBundleResult = ret;
    }

    function recordIsSimilar(bytes calldata a, bytes calldata b) external {
        bytes memory payload = abi.encodePacked(uint8(0x06), a, b);
        (bool ok, bytes memory ret) = HDC.staticcall(payload);
        require(ok, "isSimilar failed");
        lastIsSimilar = abi.decode(ret, (bool));
    }

    function recordVectorId(bytes calldata v) external {
        bytes memory payload = abi.encodePacked(uint8(0x05), v);
        (bool ok, bytes memory ret) = HDC.staticcall(payload);
        require(ok, "vectorId failed");
        lastVectorId = bytes32(ret);
    }
}
```

### 11.2 E2E Test Using Recorder

```rust
/// Deploy HdcRecorder, call recordHamming, then read lastHammingDistance
/// from contract storage and compare with the expected Rust computation.
#[test]
#[ignore = "requires HDC precompile registered in TestApplication"]
fn test_hdc_precompile_hamming_verified() {
    let config = TestConfig::default()
        .with_validators(4)
        .with_max_blocks(5)
        .with_timeout(Duration::from_secs(45));

    let deployer_key = SigningKey::from_bytes(&[0xE1; 32].into()).expect("valid key");
    let deployer = Evm::address_from_key(&deployer_key);
    let initial_balance = U256::from(10_000_000_000u64);

    // Transaction 0: deploy HdcRecorder
    let deploy_tx = Evm::sign_eip1559_create(
        &deployer_key,
        config.chain_id,
        HDC_RECORDER_BYTECODE.into(),
        0,
        5_000_000,
    );

    // Compute expected recorder contract address (CREATE: sender + nonce)
    let recorder_addr = deployer.create(0);

    // Transaction 1: call recordHamming(vec_a, vec_b)
    let vec_a = kora_hdc::HdcVector::random(42);
    let vec_b = kora_hdc::HdcVector::random(43);
    let expected_distance = kora_hdc::hamming_distance(&vec_a, &vec_b);

    let calldata = encode_record_hamming(&vec_a, &vec_b);
    let record_tx = Evm::sign_eip1559_call(
        &deployer_key,
        config.chain_id,
        recorder_addr,
        calldata,
        1,
        500_000,
    );

    let setup = TestSetup {
        genesis_alloc: vec![(deployer, initial_balance)],
        bootstrap_txs: vec![deploy_tx, record_tx],
        expected_balances: vec![],
    };

    let outcome = TestHarness::run(config, setup).expect("recorder test should pass");
    assert_eq!(outcome.blocks_finalized, 5);

    // Query contract storage slot 0 (lastHammingDistance)
    // The storage slot for a uint32 at slot 0 is keccak256(0x00...00)
    let stored_distance = outcome.query_storage(recorder_addr, U256::ZERO);
    assert_eq!(stored_distance, U256::from(expected_distance));
}
```

### 11.3 Verification Commands

```bash
# Compile the recorder contract
cd contracts && forge build

# Run existing E2E tests (single-threaded, required for QMDB)
cargo test -p kora-e2e -- --test-threads=1

# Run only HDC E2E tests
cargo test -p kora-e2e -- --test-threads=1 hdc

# Run ignored tests (slow / needs precompile registration)
cargo test -p kora-e2e -- --test-threads=1 --ignored hdc

# Run precompile unit tests
cargo test -p kora-hdc-chain -- precompile

# Run HNSW recall test
cargo test -p kora-hdc -- hnsw_recall
```

---

## 12. Test Matrix (19 Scenarios from Audit)

| # | Scenario | Type | Status | Blocker |
|---|----------|------|--------|---------|
| 1 | Hamming distance: identical vectors = 0 | Unit | DONE (precompile.rs) | -- |
| 2 | Hamming distance: random vectors ~D/2 | Unit | DONE (vector.rs) | -- |
| 3 | Bind: self-inverse property | Unit | DONE (vector.rs) | -- |
| 4 | Bundle: majority vote correctness | Unit | DONE (bundle.rs) | -- |
| 5 | Bundle: 50-vector capacity | Unit | MISSING | -- |
| 6 | Permute: full rotation identity | Unit | DONE (vector.rs) | -- |
| 7 | Serialize/deserialize roundtrip | Unit | DONE (vector.rs) | -- |
| 8 | HNSW recall >= 0.95 on 10K vectors | Integration | MISSING | -- |
| 9 | Knowledge decay half-life | Integration | DONE (knowledge.rs) | -- |
| 10 | Tier promotion lifecycle | Integration | DONE (knowledge.rs) | -- |
| 11 | Precompile bind E2E (output verified) | E2E | MISSING | AP01, AP02, AP04 |
| 12 | Precompile hamming E2E (output verified) | E2E | MISSING | AP01, AP02, AP04 |
| 13 | Precompile bundle E2E (output verified) | E2E | MISSING | AP01, AP02, AP04 |
| 14 | InsightBoard lifecycle E2E | E2E | MISSING | AP02 |
| 15 | PheromoneRegistry decay E2E | E2E | MISSING | AP02 |
| 16 | Consensus determinism with HDC ops | E2E | PARTIAL (no output check) | AP01 |
| 17 | 50ms block time | E2E | FAKE (uses 1s timeout) | AP06 |
| 18 | Concurrent stress (10K inserts, 15 tasks) | Stress | MISSING | -- |
| 19 | HNSW search benchmark < 1ms on 10K | Bench | MISSING | -- |

**Key files:**

| File | Absolute Path |
|------|--------------|
| E2E tests | `/Users/will/dev/nunchi/daeji/crates/e2e/src/tests/hdc.rs` |
| Precompile unit tests | `/Users/will/dev/nunchi/daeji/crates/hdc/chain/src/precompile.rs` |
| Core unit tests | `/Users/will/dev/nunchi/daeji/crates/hdc/core/src/vector.rs` |
| Search tests | `/Users/will/dev/nunchi/daeji/crates/hdc/core/tests/search.rs` |
| Knowledge tests | `/Users/will/dev/nunchi/daeji/crates/hdc/core/tests/knowledge.rs` |
| Stress tests | `/Users/will/dev/nunchi/daeji/crates/hdc/core/tests/stress.rs` |
| Benchmarks | `/Users/will/dev/nunchi/daeji/crates/hdc/core/benches/hdc_bench.rs` |
| Solidity HdcLib | `/Users/will/dev/nunchi/daeji/contracts/src/HdcPrecompile.sol` |
| Solidity InsightBoard | `/Users/will/dev/nunchi/daeji/contracts/src/InsightBoard.sol` |
| Test harness | `/Users/will/dev/nunchi/daeji/crates/e2e/src/harness.rs` |
| Test setup | `/Users/will/dev/nunchi/daeji/crates/e2e/src/setup.rs` |

---

## Audit Findings

> Audited 2026-05-08. Compared spec (sections 1-10) against the six implementation files
> listed below. All file paths are absolute from repo root `/Users/will/dev/nunchi/daeji/`.

### F01 -- API Signature Divergence: `HdcVector::random`

The spec assumes `HdcVector::random(&mut rng)` taking a mutable reference to a
`StdRng`. The actual implementation (line 68 of
`crates/hdc/core/src/vector.rs`) takes a bare `u64` seed and constructs
`ChaCha20Rng` internally:

```rust
pub fn random(seed: u64) -> Self        // actual
HdcVector::random(&mut StdRng::seed_from_u64(42))  // spec
```

All test files correctly use the `seed: u64` signature. The spec code examples
in sections 3.1--3.8 are **not copy-pasteable** and would not compile.

### F02 -- API Signature Divergence: Free Functions vs Methods

The spec uses method syntax throughout (`a.bind(&b)`, `a.hamming_distance(&b)`,
`v.permute(1)`, `v.complement()`, `v.anti()`, `v.to_bytes()`,
`HdcVector::from_bytes()`). The actual API uses free functions:

| Spec syntax | Actual API (in `crates/hdc/core/src/vector.rs`) |
|---|---|
| `a.bind(&b)` | `bind(&a, &b)` |
| `a.hamming_distance(&b)` | `hamming_distance(&a, &b)` |
| `v.permute(1)` | `permute(&v, 1)` |
| `v.complement()` | `complement(&v)` (private, in `crates/hdc/core/src/cognitive/affect.rs` line 140) |
| `v.anti()` | `encode_anti(&v)` (in `crates/hdc/core/src/knowledge/anti.rs` line 27) |
| `v.to_bytes()` | `serialize(&v)` (line 137) |
| `HdcVector::from_bytes()` | `deserialize(&bytes)` (line 147) |
| `HdcVector::bundle(&[...])` | `bundle(&[...])` (in `crates/hdc/core/src/bundle.rs` line 67) |

### F03 -- API Signature Divergence: Search Index Keys

Spec uses `u64` keys for `BruteForceIndex` and `HnswIndex`:

```rust
index.insert(i as u64, v.clone());
```

Actual implementation uses `H256` (`[u8; 32]`) keys via the `SearchIndex` trait
(`crates/hdc/core/src/search/mod.rs` line 33). The integration test at
`crates/hdc/core/tests/search.rs` correctly uses `H256` with a `vector_id()`
helper (lines 6-9).

### F04 -- API Signature Divergence: `HnswIndex::new`

Spec: `HnswIndex::new(/* M= */ 16, /* ef_construction= */ 200)`.
Actual: `HnswIndex::new()` takes no parameters (`crates/hdc/core/src/search/hnsw.rs`
line 124). M and ef_construction are internal constants.

### F05 -- API Signature Divergence: `KnowledgeStore`

Spec uses `KnowledgeStore::new()` (no args). Actual:
`KnowledgeStore::new(tick_duration_ms: u64)` (line 23 of
`crates/hdc/core/src/knowledge/store.rs`).

Spec's `store.tick(138.6 * 3600.0)` passes a float representing seconds.
Actual `store.tick(current_tick: u64)` takes an integer tick number
(line 170). Implementation tests convert hours to ticks manually
(e.g., 7.2 hours * 9000 ticks/hour = 64,800 ticks in
`crates/hdc/core/tests/knowledge.rs` line 36).

Spec's `store.reinforce(entry_id)` takes a single ID. Actual:
`store.reinforce(key: &[u8; 32], current_tick: u64, boost: f64)` takes three
arguments (`crates/hdc/core/src/knowledge/store.rs` line 247).

Spec's `store.search(query, k)` takes two args. Actual:
`store.search(query, k, &RetrievalContext)` takes three
(`crates/hdc/core/src/knowledge/store.rs` line 76).

### F06 -- API Signature Divergence: `KnowledgeEntry`

Spec: `KnowledgeEntry::new(vec, KnowledgeKind::Insight, 1.0)` -- 3 args.
Actual: `KnowledgeEntry::new(vec, kind, label, source, tick)` -- 5 args
(`crates/hdc/core/src/knowledge/entry.rs` line 35).

Spec references `entry.balance()` (method call). Actual: `entry.balance`
(public field).

Spec references `RetentionTier`. Actual enum is `KnowledgeTier`
(`crates/hdc/core/src/knowledge/tier.rs` line 10).

### F07 -- Missing E2E Tests: InsightBoard and PheromoneRegistry

Spec sections 5.5 and 5.6 require:
- `test_insight_board_submit_and_confirm` -- deploy InsightBoard contract,
  submit an insight, confirm it, verify state transitions.
- `test_pheromone_registry_decay` -- deploy PheromoneRegistry, deposit
  pheromone, verify decay over 10 blocks.

Neither test exists in `crates/e2e/src/tests/hdc.rs`. There are no references
to InsightBoard or PheromoneRegistry anywhere in the E2E test file. The helper
methods `TestSetup::insight_board_lifecycle()` and
`TestSetup::pheromone_registry_decay()` referenced by the spec do not exist.

### F08 -- Missing Unit Tests from Spec

The following spec-required unit tests (section 3) have no exact counterpart
in the codebase:

| Spec test | Status | Notes |
|---|---|---|
| `bundle_majority_vote` (3.2) | **Covered differently** | `bundle_result_is_similar_to_all_inputs` in `crates/hdc/core/src/bundle.rs` line 91 tests that the bundle is similar to all inputs, but does not test that `bundle(a, a, b)` is closer to `a` than `b` specifically. |
| `hamming_distance_identical_is_zero` (3.6) | **Covered** | Via `brute.rs` line 124 and `hnsw.rs` line 478, both assert self-distance == 0. Not a standalone named test. |
| `hamming_distance_complement_is_max` (3.6) | **Missing** | `complement()` is a private function in `cognitive/affect.rs`. No test verifies `hamming(v, complement(v)) == D`. The SIMD test at `search/simd.rs` line 231 (`hamming_complementary_vectors`) likely covers this but is SIMD-specific. |
| `hamming_distance_random_vectors_near_half` (3.6) | **Covered** | `random_vectors_are_quasi_orthogonal` in `vector.rs` line 198 asserts hamming distance falls in range 4900..5350. |
| `bundle_capacity_50_vectors` (3.7) | **Missing** | No test verifies that bundling 50 vectors preserves retrieval above a similarity threshold. The benchmark `bench_bundle_50` measures performance only, not correctness. |
| `anti_knowledge_double_negation` (3.8) | **Covered** | `test_encode_anti_self_inverse` in `crates/hdc/core/src/knowledge/anti.rs` line 55. |
| `anti_knowledge_orthogonal` (3.8) | **Covered** | `test_anti_orthogonal` in `crates/hdc/core/src/knowledge/anti.rs` line 63. |

### F09 -- Missing Integration Test: HNSW Recall vs Brute Force (10K vectors)

Spec section 4.2 requires `hnsw_recall_vs_brute_force` with 10,000 vectors and
100 queries, asserting recall > 0.95. The actual HNSW test
(`hnsw_matches_brute_force` in `crates/hdc/core/src/search/hnsw.rs` line 507)
uses only 200 vectors and 10 queries, and checks exact top-1 match rather than
computing a recall statistic. This directly violates anti-pattern 8.4: "Do NOT
test HNSW recall with too few vectors [...] Use at least 10,000 for recall
benchmarks."

The integration test file `crates/hdc/core/tests/search.rs` has no HNSW test at
all -- only `BruteForceIndex` tests.

### F10 -- Missing Stress Test: Concurrent Inserts and Searches

Spec section 6.2 requires `stress_concurrent_inserts_and_searches` using
`tokio::test` with 10 inserter tasks and 5 searcher tasks running concurrently
against an `Arc<RwLock<KnowledgeStore>>`. This test does not exist anywhere in
the codebase. `crates/hdc/core/tests/stress.rs` contains only the synchronous
`stress_10k_knowledge_entries` test.

### F11 -- Missing Benchmark: HNSW Search 10K

Spec section 7 requires `bench_hnsw_search_10000` in
`crates/hdc/core/benches/hdc_bench.rs`. The benchmark file (line 66-74) does
not include this benchmark. The existing benchmarks cover hamming, bind,
bundle_50, permute, brute_force_search_1000, and serialize_roundtrip. The HNSW
benchmark is absent.

Implementation added `bench_permute` (line 33) which is not in the spec but is
a reasonable addition.

### F12 -- `#[ignore]` Annotation Inconsistency

The spec's anti-pattern 8.5 and the existing E2E test convention (all tests in
`consensus.rs`, `execution.rs`, and `resilience.rs` carry `#[ignore]`) shows
that ALL E2E tests should be marked `#[ignore]`.

In `crates/e2e/src/tests/hdc.rs`:
- `test_hdc_precompile_bind` (line 94): **no `#[ignore]`**
- `test_hdc_precompile_hamming` (line 124): **no `#[ignore]`**
- `test_hdc_precompile_bundle` (line 154): **no `#[ignore]`**
- `test_hdc_consensus_determinism` (line 192): has `#[ignore]` -- correct
- `test_hdc_fast_blocks` (line 241): **no `#[ignore]`**
- `test_hdc_precompile_with_transfers` (line 264): **no `#[ignore]`**

Four of six E2E tests will run in the default `cargo test` suite and will
conflict with QMDB file partitions if run in parallel. This is exactly the
anti-pattern described in spec section 8.1.

### F13 -- 50ms Block Time Test is a No-Op

Spec section 5.8 acknowledges that `.with_block_time_ms(50)` would need to be
added to `TestConfig`. The implementation at `test_hdc_fast_blocks` (line 241
of `crates/e2e/src/tests/hdc.rs`) uses low-latency SimLink settings but does
not actually configure 50ms block times -- the harness still uses 1s
`leader_timeout`. The test comment (lines 237-239) acknowledges this limitation.
This test verifies "many blocks finalize with low latency," not 50ms block
production.

### F14 -- E2E Precompile Tests Do Not Verify Output Values

All three precompile tests (`test_hdc_precompile_bind`,
`test_hdc_precompile_hamming`, `test_hdc_precompile_bundle`) only assert that
`outcome.blocks_finalized == 3`. They do not verify:
- The precompile return value (e.g., Hamming distance is correct).
- That gas was consumed correctly.
- That the EVM execution actually succeeded (vs. silently reverting).

The harness verifies state root agreement (consensus correctness) but not
functional correctness of the precompile output.

### F15 -- Ad-Hoc Tests Beyond Spec

The implementation includes tests not described in the spec:

**In `crates/e2e/src/tests/hdc.rs`:**
- `test_hdc_precompile_bind` (line 94) -- spec only has `test_hdc_precompile_hamming_distance`.
- `test_hdc_precompile_bundle` (line 154) -- not in spec.
- `test_hdc_precompile_with_transfers` (line 264) -- mixed precompile + transfer test, not in spec.

**In `crates/hdc/core/tests/search.rs`:**
- `brute_force_1000_results_sorted` (line 39) -- verifies sort ordering of search results. Reasonable addition not in spec.

**In `crates/hdc/core/src/knowledge/store.rs` inline tests:**
- `test_tick_gc` (line 333) -- GC behavior.
- `test_tick_promotion` (line 347) -- promotion via tick.
- `test_tick_demotion` (line 365) -- demotion behavior.
- `test_persistent_never_deleted` (line 383) -- persistent tier protection.
- `test_promotion_beats_demotion` (line 401) -- priority resolution.
- `test_reinforce_resets_balance` (line 419) -- reinforce mechanics.
- `test_record_contradiction_halves_balance` (line 438) -- contradiction penalty.
- `test_search_with_anti_check` (line 451) -- anti-knowledge search.
- `test_anti_check_does_not_reject_unrelated` (line 486) -- anti false-positive check.

These are **good additions** that go beyond spec requirements and improve
coverage of the knowledge subsystem.

### F16 -- Precompile Test ABI Encoding Divergence

Spec section 5.2 uses Solidity-style ABI encoding with a 4-byte function
selector (`keccak256(b"hamming(bytes,bytes)")[..4]`). The actual implementation
uses a raw opcode byte prefix (e.g., `0x01` for hamming, `0x02` for bind,
`0x03` for bundle), matching the actual precompile implementation in
`kora-hdc-chain`. The spec's ABI encoding is wrong relative to the actual
precompile interface.

### F17 -- `Evm::sign_eip1559_call` vs `sign_eip1559_create`

The spec says to add `sign_eip1559_create` to `evm.rs`. The implementation
actually added both `sign_eip1559_create` (line 25) and `sign_eip1559_call`
(line 55). The E2E tests use `sign_eip1559_call` (line 34 of
`crates/e2e/src/tests/hdc.rs`) for precompile calls, which is cleaner than
the spec's approach of requiring the test to manually construct `TxEip1559`
structs.

---

## Implementation Status

### Unit Tests (kora-hdc) -- Spec Section 3

| Spec ID | Spec Test Name | Status | Actual Location |
|---|---|---|---|
| 3.1 | `bind_is_self_inverse` | DONE | `crates/hdc/core/src/vector.rs:209` (`bind_self_inverse`) |
| 3.2 | `bundle_majority_vote` | PARTIAL | `crates/hdc/core/src/bundle.rs:91` (`bundle_result_is_similar_to_all_inputs`) -- tests similarity to all inputs but not the "closer to a than b" property |
| 3.3 | `permute_rotation` | DONE | `crates/hdc/core/src/vector.rs:236` (`permute_full_rotation_is_identity`) |
| 3.4a | `determinism_same_seed_same_vector` | DONE | `crates/hdc/core/src/vector.rs:173` (`random_is_deterministic`) |
| 3.4b | `determinism_different_seed_different_vector` | DONE | `crates/hdc/core/src/vector.rs:178` (`random_different_seeds_differ`) |
| 3.5 | `serialize_deserialize_roundtrip` | DONE | `crates/hdc/core/src/vector.rs:254` (`serialize_deserialize_roundtrip`) |
| 3.6a | `hamming_distance_identical_is_zero` | DONE (indirect) | Asserted in multiple search tests (distance to self == 0) |
| 3.6b | `hamming_distance_complement_is_max` | MISSING | `complement()` is private; no public test exists |
| 3.6c | `hamming_distance_random_vectors_near_half` | DONE | `crates/hdc/core/src/vector.rs:198` (`random_vectors_are_quasi_orthogonal`) |
| 3.7 | `bundle_capacity_50_vectors` | MISSING | No correctness test for 50-vector bundle capacity |
| 3.8a | `anti_knowledge_double_negation` | DONE | `crates/hdc/core/src/knowledge/anti.rs:55` (`test_encode_anti_self_inverse`) |
| 3.8b | `anti_knowledge_orthogonal` | DONE | `crates/hdc/core/src/knowledge/anti.rs:63` (`test_anti_orthogonal`) |

### Integration Tests (kora-hdc) -- Spec Section 4

| Spec ID | Spec Test Name | Status | Actual Location |
|---|---|---|---|
| 4.1 | `brute_force_insert_1000_search_top_k` | DONE | `crates/hdc/core/tests/search.rs:12` |
| 4.2 | `hnsw_recall_vs_brute_force` (10K, recall > 0.95) | MISSING | `crates/hdc/core/src/search/hnsw.rs:507` exists but uses 200 vectors, not 10K; no recall metric |
| 4.3 | `knowledge_decay_reduces_balance` | DONE | `crates/hdc/core/tests/knowledge.rs:25` |
| 4.4 | `tier_promotion_transient_to_working_to_consolidated` | DONE | `crates/hdc/core/tests/knowledge.rs:51` |

### E2E Tests (kora-e2e) -- Spec Section 5

| Spec ID | Spec Test Name | Status | Actual Location |
|---|---|---|---|
| 5.4 | `test_hdc_precompile_hamming_distance` | DONE | `crates/e2e/src/tests/hdc.rs:124` (`test_hdc_precompile_hamming`) -- no output verification |
| 5.5 | `test_insight_board_submit_and_confirm` | MISSING | No InsightBoard contract deployment or lifecycle test |
| 5.6 | `test_pheromone_registry_decay` | MISSING | No PheromoneRegistry test |
| 5.7 | `test_consensus_determinism_with_hdc` | DONE | `crates/e2e/src/tests/hdc.rs:193` (`test_hdc_consensus_determinism`) |
| 5.8 | `test_50ms_block_time` | PARTIAL | `crates/e2e/src/tests/hdc.rs:241` (`test_hdc_fast_blocks`) -- does not actually test 50ms block time |

### Stress Tests -- Spec Section 6

| Spec ID | Spec Test Name | Status | Actual Location |
|---|---|---|---|
| 6.1 | `stress_10k_knowledge_entries` | DONE | `crates/hdc/core/tests/stress.rs:13` |
| 6.2 | `stress_concurrent_inserts_and_searches` | MISSING | No concurrent test exists |

### Benchmarks -- Spec Section 7

| Spec Benchmark | Status | Actual Location |
|---|---|---|
| `bench_hamming_distance` | DONE | `crates/hdc/core/benches/hdc_bench.rs:6` |
| `bench_bind` | DONE | `crates/hdc/core/benches/hdc_bench.rs:15` |
| `bench_bundle_50` | DONE | `crates/hdc/core/benches/hdc_bench.rs:24` |
| `bench_brute_force_search_1000` | DONE | `crates/hdc/core/benches/hdc_bench.rs:41` |
| `bench_hnsw_search_10000` | MISSING | Not present in benchmark file |
| `bench_serialize_roundtrip` | DONE | `crates/hdc/core/benches/hdc_bench.rs:55` |

### Infrastructure -- Spec Section 5.1 / Checklist

| Item | Status | Notes |
|---|---|---|
| Add `hdc` module to `mod.rs` | DONE | `crates/e2e/src/tests/mod.rs:5` |
| Add `kora-hdc-chain` dependency | DONE | `crates/e2e/Cargo.toml:17` |
| Add `sign_eip1559_create` helper | DONE | `crates/node/domain/src/evm.rs:25` |
| Add `sign_eip1559_call` helper | DONE (beyond spec) | `crates/node/domain/src/evm.rs:55` |
| Criterion benchmark harness | DONE | `crates/hdc/core/benches/hdc_bench.rs` exists |
| InsightBoard contract compilation | MISSING | No compiled bytecode, no deployment helper |
| PheromoneRegistry contract compilation | MISSING | No compiled bytecode, no deployment helper |

---

## Anti-Patterns & Duct Tape

### AP01 -- Missing `#[ignore]` on E2E Tests (Violates Spec 8.5)

**File:** `crates/e2e/src/tests/hdc.rs`
**Lines:** 94, 124, 154, 241, 264

Four of six E2E tests lack `#[ignore]`. Every other E2E test file in
`crates/e2e/src/tests/` marks all tests with `#[ignore]`. Running
`cargo test -p kora-e2e` without `--test-threads=1` will cause QMDB file
conflicts.

### AP02 -- E2E Tests Assert Block Count Only, Not Functional Correctness

**File:** `crates/e2e/src/tests/hdc.rs`
**Lines:** 118, 148, 184

All precompile tests end with `assert_eq!(outcome.blocks_finalized, 3)`. They
confirm that the transaction did not crash the network and that validators
agreed on state, but they do not assert that the precompile computed the
correct Hamming distance, the correct bind result, or the correct bundle. A
silently reverting or incorrect precompile would pass these tests.

### AP03 -- HNSW Recall Test Uses 200 Vectors (Violates Spec 8.4)

**File:** `crates/hdc/core/src/search/hnsw.rs`
**Lines:** 507-539 (`hnsw_matches_brute_force`)

The spec explicitly warns: "Testing with fewer than 1,000 vectors gives
misleading recall numbers. Use at least 10,000." The inline test uses 200
vectors. This is the exact anti-pattern the spec calls out.

### AP04 -- Spec Code Examples Are Not Compilable

Every code snippet in spec sections 3.1-3.8, 4.1-4.4, and 6.1-6.2 uses API
signatures that do not match the actual codebase (method syntax vs free
functions, `StdRng` vs `u64` seeds, `u64` keys vs `H256`, different
constructors). An implementer following the spec verbatim would get compile
errors on every test.

### AP05 -- No Negative Testing for Precompile Edge Cases

No E2E test submits invalid data to the HDC precompile (malformed input,
wrong-length vectors, zero-length data, enormous payloads). The precompile
should reject bad input gracefully rather than panicking.

### AP06 -- `test_hdc_fast_blocks` Claims to Test 50ms Blocks But Does Not

**File:** `crates/e2e/src/tests/hdc.rs`
**Lines:** 237-258

The test comment honestly admits the limitation, but the test name is
misleading. It tests "many blocks with low network latency," not 50ms block
production. The `with_block_time_ms()` builder does not exist on `TestConfig`.

### AP07 -- Stress Test Does Not Assert Entry Count

**File:** `crates/hdc/core/tests/stress.rs`
**Line:** 32

The stress test correctly calls `assert_eq!(store.len(), 10_000)` -- this is
fine. However, it constructs `query = HdcVector::random(5000)` on line 35 and
expects it to match the entry inserted with seed `5000`. This works because
`HdcVector::random` is deterministic, but the test relies on an implicit
invariant (same seed = same vector = distance 0 in search) that is not
documented in the test body.

---

## Recommended Changes Checklist

### Priority 1 -- Missing Tests That Block Spec Compliance

- [ ] **RC01** -- Add `#[ignore]` to all E2E tests in `crates/e2e/src/tests/hdc.rs` that lack it (lines 94, 124, 154, 241, 264). Annotation: `#[ignore = "flaky when run in parallel - run with --test-threads=1"]`.
- [ ] **RC02** -- Add `hnsw_recall_vs_brute_force` integration test in `crates/hdc/core/tests/search.rs` with 10,000 vectors, 100 queries, `recall > 0.95` threshold. Use `HnswIndex` from `kora_hdc::search`.
- [ ] **RC03** -- Add `stress_concurrent_inserts_and_searches` in `crates/hdc/core/tests/stress.rs`. Use `Arc<tokio::sync::RwLock<KnowledgeStore>>`, 10 inserter tasks (1000 each), 5 searcher tasks. Requires adding `tokio` as a dev-dependency to `kora-hdc` with `rt-multi-thread` and `macros` features.
- [ ] **RC04** -- Add `bench_hnsw_search_10000` to `crates/hdc/core/benches/hdc_bench.rs`. Insert 10,000 vectors into `HnswIndex`, benchmark `search(&query, 10)`.
- [ ] **RC05** -- Add `bundle_capacity_50_vectors` unit test in `crates/hdc/core/src/bundle.rs` inline test module. Bundle 50 random vectors, assert each constituent has hamming distance below `THRESHOLD_HAMMING` to the bundle.
- [ ] **RC06** -- Add `hamming_distance_complement_is_max` test. Either make `complement()` public or implement it inline in the test. Assert `hamming_distance(&v, &complement(v)) == D`.

### Priority 2 -- E2E Test Quality Improvements

- [ ] **RC07** -- Add output verification to `test_hdc_precompile_hamming`. After the harness run, query the receipt or state to verify the returned Hamming distance matches the locally computed `hamming_distance(&vec_a, &vec_b)`. This requires either receipt introspection in the harness or a wrapper contract that stores the result.
- [ ] **RC08** -- Add negative/edge-case E2E test `test_hdc_precompile_invalid_input`. Send malformed data to `0x09` (e.g., 1 byte, 1279 bytes, empty). Verify the transaction reverts without crashing the network.
- [ ] **RC09** -- Add `test_insight_board_submit_and_confirm` E2E test. Requires compiling InsightBoard.sol, embedding bytecode, creating `TestSetup::insight_board_lifecycle()` helper on `TestSetup`, and deploying via `sign_eip1559_create`.
- [ ] **RC10** -- Add `test_pheromone_registry_decay` E2E test. Same contract compilation/embedding requirement as RC09 but for PheromoneRegistry.
- [ ] **RC11** -- Rename `test_hdc_fast_blocks` to `test_hdc_high_throughput_blocks` or add `with_block_time_ms(50)` to `TestConfig` and actually test 50ms block production.

### Priority 3 -- Spec Document Corrections

- [ ] **RC12** -- Update all spec code examples in sections 3.1--3.8, 4.1--4.4, and 6.1--6.2 to use the actual API: `HdcVector::random(seed)`, free functions `bind()`, `hamming_distance()`, `permute()`, `serialize()`/`deserialize()`, `H256` keys, and correct `KnowledgeStore`/`KnowledgeEntry` constructor signatures.
- [ ] **RC13** -- Update spec section 5.2 to document the actual opcode-based precompile interface (opcode byte prefix, not Solidity ABI selectors).
- [ ] **RC14** -- Update spec `HnswIndex::new()` constructor to reflect zero-argument signature.
- [ ] **RC15** -- Add the ad-hoc tests (bind, bundle precompile, mixed transfer test, brute force sort order, knowledge store lifecycle tests) to the spec checklist so they are tracked going forward.

---

## Second-Pass Remediation Detail

> Second-pass scope: `crates/e2e/src/tests/hdc.rs`, the E2E harness/setup/node
> files, HDC precompile/provider code, Solidity contract tests/sources, HNSW
> search tests, stress tests, and benchmark coverage. This section is an
> implementation-ready remediation plan; it does not change code.

### Harness Preconditions

Fix these before treating any HDC E2E result as meaningful:

1. Enable the HDC precompile in every E2E executor path.
   - `TestApplication::new` currently constructs `RevmExecutor::new(1337)`.
   - `FinalizedReporter` currently receives `RevmExecutor::new(chain_id)`.
   - Both must use `RevmExecutor::new(chain_id).with_hdc_precompile()`.
   - Thread `chain_id` and `gas_limit` into `TestApplication`; do not hardcode
     `1337` or `30_000_000` there.

2. Add one observation mechanism for E2E assertions beyond block count.
   - Preferred: expose receipts from finalized execution through `TestOutcome`
     or a harness-local `BlockIndex`, including `success`, `gas_used`, logs,
     and `contract_address`.
   - Also add a read-only call helper, e.g. `TestNode::simulate_call_at(head,
     CallParams) -> Bytes`, backed by `RevmExecutor::simulate_call`.
   - For raw precompile transaction outputs, use a small `HdcRecorder`
     contract that calls `0x09` and stores either the scalar result or
     `keccak256(output)` in storage. Direct transaction return data is not
     persisted by the EVM receipt model.

3. Reconcile Solidity `HdcLib` with the actual Rust precompile interface before
   contract E2E tests.
   - Rust `kora-hdc-chain` opcodes are: `0x01` hamming, `0x02` bind, `0x03`
     bundle, `0x04` permute, `0x05` vector_id, `0x06` is_similar.
   - `contracts/src/HdcPrecompile.sol` currently documents/calls `0x01`
     storeVector, `0x02` searchSimilar, `0x03` deleteVector, `0x04` bundle
     with a `uint16` count, `0x05` bind, `0x06` hamming, and `0x07` permute.
   - `InsightBoard.submit()` depends on `searchSimilar()` and `storeVector()`,
     so real InsightBoard E2E tests cannot pass until either Rust supports that
     contract ABI or the Solidity library/contracts are changed to the Rust ABI.

### Concrete Test Matrix

| Area | Test | Setup | Assertions | Marking |
|---|---|---|---|---|
| Precompile output | `test_hdc_precompile_hamming_output` | 4 validators, 3 blocks, funded caller, vectors seeds `100`/`101`; call recorder -> opcode `0x01` | Receipt success; stored `u32` equals `hamming_distance(&a, &b)` decoded from the low 4 bytes of the 32-byte return; finalized state root/seed converge | `#[ignore]`, serial |
| Precompile output | `test_hdc_precompile_bind_output` | Vectors seeds `42`/`43`; recorder calls opcode `0x02` | Receipt success; stored output hash equals `keccak256(serialize(&bind(&a, &b)))`; gas used is at least tx base plus precompile gas | `#[ignore]`, serial |
| Precompile output | `test_hdc_precompile_bundle_output` | Vectors seeds `200`/`201`/`202`; recorder calls opcode `0x03` with `u32` count prefix | Receipt success; stored output hash equals `keccak256(serialize(&bundle(&[&a, &b, &c])))` | `#[ignore]`, serial |
| Precompile output | `test_hdc_precompile_permute_vector_id_is_similar` | One recorder contract function per opcode `0x04`, `0x05`, `0x06` | Permute hash equals local `permute(&v, n)`; vector id equals local `vector_id(&v)`; identical vectors return `is_similar == true`, random far vectors return `false` unless below `THRESHOLD_HAMMING` | `#[ignore]`, serial |
| Mixed E2E | `test_hdc_precompile_with_transfers_checks_output` | Existing mixed precompile + transfer scenario, but via recorder | Receiver balance equals transfer amount; sender balance changed consistently; recorder output hash matches local bind; all nodes finalize the same head | `#[ignore]`, serial |
| Determinism | `test_hdc_consensus_determinism` | Existing two-run same-seed setup with bind/hamming/bundle calls | `state_root`, `seed`, `finalized_head`, and per-node finalization counts match across runs; no receipt failure in either run | `#[ignore]`, serial |
| Negative precompile | `test_hdc_precompile_invalid_inputs_revert` | Submit malformed direct or recorder calls | Empty input, invalid opcode `0xff`, one short vector (`BYTES - 1`), malformed bundle count, and too-low gas all produce failed receipts or caught recorder failures; network still finalizes the target blocks | `#[ignore]`, serial |
| Negative precompile | `test_hdc_invalid_tx_does_not_poison_following_tx` | Invalid HDC call at nonce 0 followed by valid transfer or valid recorder call at nonce 1 | Invalid receipt fails; subsequent valid tx succeeds if nonce semantics allow inclusion, or document/verify mempool behavior if failed nonce blocks later txs; validators keep converging | `#[ignore]`, serial |
| Contract E2E | `test_insight_board_submit_and_confirm` | Deploy real `InsightBoard`; submit 1280-byte vector/content with `0.01 ether`; confirm from second account | Contract address captured; `submit` receipt success; `InsightPublished` and state-change logs present; `insights(id)` shows author, `SUBMITTED`, `TRANSIENT`, stake; after confirm, state `ACTIVE`, confirmations `1` | `#[ignore]`, serial; blocked by HdcLib/precompile ABI mismatch |
| Contract E2E | `test_insight_board_negative_cases` | Same deployed board | Short vector reverts; insufficient stake reverts; double confirm reverts; challenge with non-anti insight reverts; receipts/logs prove failure mode without consensus stall | `#[ignore]`, serial; blocked by HdcLib/precompile ABI mismatch |
| Contract E2E | `test_pheromone_registry_decay` | Deploy `PheromoneRegistry`; deposit THREAT vector with intensity `1000` and `0.001 ether`; run to block offsets | `currentIntensity(id)` returns `1000` at deposit, `500` after 100 blocks, `250` after 200 blocks; `cleanup` succeeds only after death threshold | `#[ignore]`, serial |
| Contract E2E | `test_pheromone_registry_negative_cases` | Same deployed registry | Short vector, intensity `<100`, intensity `>10_000`, insufficient stake, double confirm, confirm-dead, cleanup-alive all revert with failed receipts | `#[ignore]`, serial |
| 50ms blocks | `test_hdc_50ms_block_time_observed` | Add `TestConfig::with_block_time_ms(50)` and thread it into `simplex::Config` timeouts; 4 validators, 20 blocks, 1-5ms sim link latency | `blocks_finalized == 20`; recorded finalization timestamps on node 0 have median inter-block interval <= 100ms and p95 <= 250ms in CI; elapsed wall time for 20 blocks <= 5s; test fails if it only changes SimLink latency | `#[ignore]`, serial |
| HNSW recall | `hnsw_recall_vs_brute_force_10k` | 10,000 deterministic vectors in both `BruteForceIndex` and `HnswIndex`; `hnsw.set_ef_search(200)`; 100 deterministic query seeds; `top_k = 10` | Recall@10 = `intersection(hnsw_top10, brute_top10) / 1000` is >= `0.95`; top-1 exact hit rate is >= `0.90`; every result list is sorted by distance then key | normal if runtime is acceptable; otherwise `#[ignore = "slow recall test"]` |
| HNSW stress | `stress_concurrent_inserts_and_searches` | `Arc<tokio::sync::RwLock<KnowledgeStore>>`; 10 inserters x 1000 entries; 5 searchers x 100 queries with barriers | Final `store.len() == 10_000`; no task panics; all returned result windows are sorted; seeded self-query after inserts returns distance `0`; completes under 60s in CI | `#[ignore = "slow stress test"]` if flaky/slow |
| HNSW benchmark | `bench_hnsw_search_10000` | Criterion bench with 10,000 inserted vectors, deterministic query, `top_k = 10`, `ef_search = 100` and optional `ef_search = 200` group | Median search target `< 1 ms`; no >20% regression against checked-in Criterion baseline without review; report build time separately if added | benchmark only |

### Ignored and Serial Strategy

All tests in `crates/e2e/src/tests/hdc.rs` should be explicitly ignored:

```rust
#[ignore = "e2e consensus test; run serially with --test-threads=1"]
```

Run them intentionally and serially because each test starts a multi-validator
runtime and QMDB-backed ledgers:

```bash
cargo test -p kora-e2e hdc -- --ignored --test-threads=1
cargo test -p kora-e2e test_hdc_precompile_hamming_output -- --ignored --test-threads=1
```

If using nextest, keep process concurrency at one and run ignored tests
explicitly:

```bash
cargo nextest run -p kora-e2e --run-ignored ignored-only -j 1
```

Do not rely on the default `cargo test -p kora-e2e`; ignored tests will be
skipped, and non-ignored E2E tests can compete for local resources.

### Output-Value Assertion Detail

Raw precompile E2E tests need assertions that are independent of consensus
agreement:

- Hamming: decode the 32-byte return as a big-endian word and compare
  `u32::from_be_bytes(output[28..32])` to local `hamming_distance(&a, &b)`.
- Bind: compare `keccak256(output)` to `keccak256(serialize(&bind(&a, &b)))`.
- Bundle: compare `keccak256(output)` to
  `keccak256(serialize(&bundle(&[&a, &b, &c])))`.
- Permute: compare `keccak256(output)` to `keccak256(serialize(&permute(&v, n)))`.
- Vector ID: compare the 32-byte output directly to `vector_id(&v)`.
- Similarity: assert the last byte of the 32-byte word is `1` for identical
  vectors and `0` for a known far pair whose local distance is above
  `THRESHOLD_HAMMING`.

The current `TestOutcome` cannot prove any of this by itself. Add receipt/log
capture, a read-only call helper, or the recorder contract storage pattern
before replacing block-count-only assertions.

### 50ms Block Verification Detail

`test_hdc_fast_blocks` is not a 50ms block test because it only lowers simulated
network latency. The real remediation is:

1. Add `block_time_ms: u64` to `TestConfig`, defaulting to the current behavior.
2. Add `with_block_time_ms(50)`.
3. Thread the value into `simplex::Config` in `start_single_node`.
4. Record finalization `Instant`s in `wait_for_finalized_head` for at least one
   node, preferably all nodes.
5. Assert observed timing with CI-tolerant bounds:
   - `blocks_finalized == 20`.
   - Median inter-finalization interval for node 0 is `<= 100ms`.
   - P95 inter-finalization interval is `<= 250ms`.
   - Total wall-clock time for 20 blocks is `<= 5s`.

If the consensus engine finalizes faster than the configured timeout, rename
the test to `test_hdc_high_throughput_blocks` and keep a separate test for any
actual scheduler/interval mechanism.

### Contract E2E Detail

Use Foundry tests as behavioral templates, but do not copy their
`TestableInsightBoard` shortcut into E2E. E2E must deploy real bytecode and run
against the real precompile path.

Required contract E2E harness support:

- Compile bytecode with Foundry and embed or load the JSON artifact.
- Deploy with `Evm::sign_eip1559_create`.
- Capture the creation receipt `contract_address`, or precompute CREATE
  addresses from `(sender, nonce)` and assert against the receipt.
- Call contract functions with ABI-encoded calldata via `sign_eip1559_call`.
- Use receipts/logs for event assertions and `simulate_call_at` for view
  functions such as `insights(id)`, `currentIntensity(id)`, and `sinr(id)`.

Contract-specific cases:

- `InsightBoard`: submit, confirm, tier promotion at 3/10/25 confirmations,
  challenge with anti-knowledge, challenge resolution after 5 confirmations,
  renew only when archived, purge only when purgeable. This is blocked until
  `HdcLib` and the Rust precompile agree on store/search/delete semantics.
- `PheromoneRegistry`: deposit happy path, type-specific half-life decay
  (`THREAT=100`, `OPPORTUNITY=250`, `WISDOM=1000` blocks), confirmation reducing
  half-life, SINR with same-location same-type interference, cleanup of dead
  pheromones, and negative cases for invalid vector/intensity/stake/double
  confirm/alive cleanup.

### HNSW Recall, Stress, and Benchmark Thresholds

Use deterministic seeds so failures are reproducible:

| Check | Minimum Data Size | Threshold |
|---|---:|---|
| HNSW recall@10 vs brute force | 10,000 vectors, 100 queries | `>= 0.95` |
| HNSW exact top-1 hit rate | 10,000 vectors, 100 queries | `>= 0.90` |
| HNSW deterministic rebuild | 1,000 vectors, 20 queries | exact result equality across two builds |
| Concurrent knowledge stress | 10,000 entries, 15 tasks | final len `10_000`, no panics, sorted results |
| `bench_hnsw_search_10000` | 10,000 vectors | median `< 1 ms` |
| Existing `bench_brute_force_search_1000` | 1,000 vectors | median `< 100 us` |
| Existing `bench_bundle_50` | 50 vectors | median `< 5 us` |

If recall does not meet `0.95` with `ef_search=100`, raise the test beam to
`200` via `set_ef_search(200)` before relaxing the threshold. A lower threshold
should be treated as a product decision, not a test convenience.

### Commands

Use these commands for the remediation pass:

```bash
# Rust formatting and lints
cargo +nightly fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings

# HDC unit/integration tests
cargo test -p kora-hdc
cargo test -p kora-hdc --test search hnsw_recall_vs_brute_force_10k
cargo test -p kora-hdc --test stress stress_concurrent_inserts_and_searches -- --ignored

# E2E HDC tests, intentionally ignored and serial
cargo test -p kora-e2e hdc -- --ignored --test-threads=1

# Solidity contract tests and artifacts
(cd contracts && forge test)
(cd contracts && forge build)
(cd contracts && forge inspect src/InsightBoard.sol:InsightBoard bytecode)
(cd contracts && forge inspect src/PheromoneRegistry.sol:PheromoneRegistry bytecode)

# Benchmarks
cargo bench -p kora-hdc --bench hdc_bench

# Full repo gate
just ci
```
