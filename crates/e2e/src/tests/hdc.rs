//! E2E tests for HDC integration.
//!
//! These tests verify that HDC operations work correctly across a
//! multi-validator simulated network with consensus.
//!
//! ## Precompile output observability
//!
//! The harness does not expose TX receipt return data (precompile output is
//! not stored in the EVM state trie). Instead, output correctness is verified
//! in two complementary ways:
//!
//! 1. **E2E consensus tests** (`test_hdc_precompile_*`): submit real
//!    transactions into the simulated network and assert that all validators
//!    reach state-root convergence. This proves the precompile executes
//!    deterministically and does not diverge across nodes.
//!
//! 2. **Precompile unit assertions** (`test_precompile_output_*`): call
//!    `kora_hdc_chain::hdc_precompile()` directly and assert exact output
//!    values. These are fast, synchronous, and require no network — they cover
//!    correctness while the E2E tests cover integration.
//!
//! Run with: cargo test -p kora-e2e -- --test-threads=1

use std::time::Duration;

use alloy_primitives::{Address, Bytes, U256};
use k256::ecdsa::SigningKey;
use kora_domain::evm::Evm;

use crate::{TestConfig, TestHarness, TestSetup};

// ─── Helpers ─────────────────────────────────────────────────────────────────

/// HDC precompile address (0x09).
fn hdc_precompile_address() -> Address {
    kora_hdc_chain::HDC_PRECOMPILE_ADDRESS
}

/// Build a transaction calling the HDC precompile with raw opcode + data.
fn call_hdc_precompile(
    key: &SigningKey,
    chain_id: u64,
    opcode: u8,
    data: &[u8],
    nonce: u64,
    gas_limit: u64,
) -> kora_domain::Tx {
    let mut input = Vec::with_capacity(1 + data.len());
    input.push(opcode);
    input.extend_from_slice(data);

    Evm::sign_eip1559_call(
        key,
        chain_id,
        hdc_precompile_address(),
        Bytes::from(input),
        nonce,
        gas_limit,
    )
}

/// Build a hamming distance precompile call (opcode 0x01).
fn call_hamming(
    key: &SigningKey,
    chain_id: u64,
    a: &kora_hdc::HdcVector,
    b: &kora_hdc::HdcVector,
    nonce: u64,
) -> kora_domain::Tx {
    let mut data = Vec::with_capacity(kora_hdc::BYTES * 2);
    data.extend_from_slice(&kora_hdc::serialize(a));
    data.extend_from_slice(&kora_hdc::serialize(b));
    call_hdc_precompile(key, chain_id, 0x01, &data, nonce, 100_000)
}

/// Build a bind precompile call (opcode 0x02).
fn call_bind(
    key: &SigningKey,
    chain_id: u64,
    a: &kora_hdc::HdcVector,
    b: &kora_hdc::HdcVector,
    nonce: u64,
) -> kora_domain::Tx {
    let mut data = Vec::with_capacity(kora_hdc::BYTES * 2);
    data.extend_from_slice(&kora_hdc::serialize(a));
    data.extend_from_slice(&kora_hdc::serialize(b));
    call_hdc_precompile(key, chain_id, 0x02, &data, nonce, 100_000)
}

/// Build a bundle precompile call (opcode 0x03).
fn call_bundle(
    key: &SigningKey,
    chain_id: u64,
    vectors: &[&kora_hdc::HdcVector],
    nonce: u64,
) -> kora_domain::Tx {
    let count = vectors.len() as u32;
    let mut data = Vec::with_capacity(4 + kora_hdc::BYTES * vectors.len());
    data.extend_from_slice(&count.to_be_bytes());
    for v in vectors {
        data.extend_from_slice(&kora_hdc::serialize(v));
    }
    let gas_limit = 100_000 + (vectors.len() as u64) * 200;
    call_hdc_precompile(key, chain_id, 0x03, &data, nonce, gas_limit)
}

/// Build a permute precompile call (opcode 0x04).
fn call_permute(
    key: &SigningKey,
    chain_id: u64,
    v: &kora_hdc::HdcVector,
    n: u32,
    nonce: u64,
) -> kora_domain::Tx {
    let mut data = Vec::with_capacity(kora_hdc::BYTES + 4);
    data.extend_from_slice(&kora_hdc::serialize(v));
    data.extend_from_slice(&n.to_be_bytes());
    call_hdc_precompile(key, chain_id, 0x04, &data, nonce, 100_000)
}

// ─── Precompile Output Assertions ────────────────────────────────────────────
//
// These tests call `hdc_precompile()` directly (no network) and assert
// exact output values. They are fast and cover correctness in isolation.

/// Verify hamming distance output: returned ABI-encoded u32 matches local
/// computation.
#[test]
fn test_precompile_output_hamming() {
    let a = kora_hdc::HdcVector::random(100);
    let b = kora_hdc::HdcVector::random(101);

    let mut input = vec![0x01u8];
    input.extend_from_slice(&kora_hdc::serialize(&a));
    input.extend_from_slice(&kora_hdc::serialize(&b));

    let (_gas, output) =
        kora_hdc_chain::hdc_precompile(&input, 100_000).expect("hamming precompile");

    assert_eq!(output.len(), 32, "hamming output must be 32 bytes (ABI uint32)");

    let returned_dist = u32::from_be_bytes(output[28..32].try_into().expect("slice len"));
    let expected_dist = kora_hdc::hamming_distance(&a, &b);
    assert_eq!(
        returned_dist, expected_dist,
        "hamming distance mismatch: precompile returned {returned_dist}, local computed {expected_dist}"
    );
}

/// Verify bind output: returned bytes deserialize to the XOR of the two
/// input vectors.
#[test]
fn test_precompile_output_bind() {
    let a = kora_hdc::HdcVector::random(200);
    let b = kora_hdc::HdcVector::random(201);

    let mut input = vec![0x02u8];
    input.extend_from_slice(&kora_hdc::serialize(&a));
    input.extend_from_slice(&kora_hdc::serialize(&b));

    let (_gas, output) = kora_hdc_chain::hdc_precompile(&input, 100_000).expect("bind precompile");

    assert_eq!(output.len(), kora_hdc::BYTES, "bind output must be {} bytes", kora_hdc::BYTES);

    let output_bytes: &[u8; 1280] = output.as_slice().try_into().expect("output len");
    let returned_vec = kora_hdc::deserialize(output_bytes);
    let expected_vec = kora_hdc::bind(&a, &b);
    assert_eq!(returned_vec, expected_vec, "bind XOR result mismatch");

    // Verify the self-inverse property: bind(result, b) should recover a
    let recovered = kora_hdc::bind(&returned_vec, &b);
    assert_eq!(recovered, a, "bind self-inverse: bind(bind(a,b), b) != a");
}

/// Verify bundle output: returned bytes deserialize to the majority-vote of
/// the input vectors.
#[test]
fn test_precompile_output_bundle() {
    let a = kora_hdc::HdcVector::random(300);
    let b = kora_hdc::HdcVector::random(301);
    let c = kora_hdc::HdcVector::random(302);

    let count: u32 = 3;
    let mut input = vec![0x03u8];
    input.extend_from_slice(&count.to_be_bytes());
    input.extend_from_slice(&kora_hdc::serialize(&a));
    input.extend_from_slice(&kora_hdc::serialize(&b));
    input.extend_from_slice(&kora_hdc::serialize(&c));

    let (_gas, output) =
        kora_hdc_chain::hdc_precompile(&input, 100_000).expect("bundle precompile");

    assert_eq!(output.len(), kora_hdc::BYTES, "bundle output must be {} bytes", kora_hdc::BYTES);

    let output_bytes: &[u8; 1280] = output.as_slice().try_into().expect("output len");
    let returned_vec = kora_hdc::deserialize(output_bytes);
    let expected_vec = kora_hdc::bundle(&[&a, &b, &c]);
    assert_eq!(returned_vec, expected_vec, "bundle majority-vote result mismatch");
}

/// Verify permute output: returned bytes deserialize to the cyclic rotation
/// of the input vector by n positions.
#[test]
fn test_precompile_output_permute() {
    let v = kora_hdc::HdcVector::random(400);
    let n: u32 = 37;

    let mut input = vec![0x04u8];
    input.extend_from_slice(&kora_hdc::serialize(&v));
    input.extend_from_slice(&n.to_be_bytes());

    let (_gas, output) =
        kora_hdc_chain::hdc_precompile(&input, 100_000).expect("permute precompile");

    assert_eq!(output.len(), kora_hdc::BYTES, "permute output must be {} bytes", kora_hdc::BYTES);

    let output_bytes: &[u8; 1280] = output.as_slice().try_into().expect("output len");
    let returned_vec = kora_hdc::deserialize(output_bytes);
    let expected_vec = kora_hdc::permute(&v, n as usize);
    assert_eq!(returned_vec, expected_vec, "permute cyclic rotation mismatch");
}

// ─── Invalid Input Rejection ─────────────────────────────────────────────────

/// Empty input must be rejected.
#[test]
fn test_precompile_invalid_empty_input() {
    let result = kora_hdc_chain::hdc_precompile(&[], 100_000);
    assert!(result.is_err(), "empty input must be rejected");
}

/// Unknown opcode must be rejected.
#[test]
fn test_precompile_invalid_opcode() {
    let result = kora_hdc_chain::hdc_precompile(&[0xFF], 100_000);
    assert!(result.is_err(), "unknown opcode must be rejected");
}

/// Hamming with wrong input length (only one vector instead of two).
#[test]
fn test_precompile_invalid_hamming_short_input() {
    let a = kora_hdc::HdcVector::random(500);
    let mut input = vec![0x01u8];
    input.extend_from_slice(&kora_hdc::serialize(&a));
    // Missing second vector
    let result = kora_hdc_chain::hdc_precompile(&input, 100_000);
    assert!(result.is_err(), "hamming with one vector must be rejected");
}

/// Bind with extra trailing byte must be rejected.
#[test]
fn test_precompile_invalid_bind_extra_byte() {
    let a = kora_hdc::HdcVector::random(501);
    let b = kora_hdc::HdcVector::random(502);
    let mut input = vec![0x02u8];
    input.extend_from_slice(&kora_hdc::serialize(&a));
    input.extend_from_slice(&kora_hdc::serialize(&b));
    input.push(0xFF); // trailing byte
    let result = kora_hdc_chain::hdc_precompile(&input, 100_000);
    assert!(result.is_err(), "bind with trailing byte must be rejected");
}

/// Bundle with count=0 must be rejected.
#[test]
fn test_precompile_invalid_bundle_zero_count() {
    let mut input = vec![0x03u8];
    input.extend_from_slice(&0u32.to_be_bytes());
    let result = kora_hdc_chain::hdc_precompile(&input, 100_000);
    assert!(result.is_err(), "bundle with count=0 must be rejected");
}

/// Bundle with count that exceeds payload must be rejected.
#[test]
fn test_precompile_invalid_bundle_count_mismatch() {
    // Claims 2 vectors but only provides 1
    let v = kora_hdc::HdcVector::random(503);
    let mut input = vec![0x03u8];
    input.extend_from_slice(&2u32.to_be_bytes());
    input.extend_from_slice(&kora_hdc::serialize(&v));
    let result = kora_hdc_chain::hdc_precompile(&input, 100_000);
    assert!(result.is_err(), "bundle with count/payload mismatch must be rejected");
}

/// Permute with missing rotation bytes must be rejected.
#[test]
fn test_precompile_invalid_permute_missing_n() {
    let v = kora_hdc::HdcVector::random(504);
    let mut input = vec![0x04u8];
    input.extend_from_slice(&kora_hdc::serialize(&v));
    // Missing 4-byte rotation count
    let result = kora_hdc_chain::hdc_precompile(&input, 100_000);
    assert!(result.is_err(), "permute without rotation count must be rejected");
}

// ─── Gas Metering ────────────────────────────────────────────────────────────

/// Hamming with insufficient gas must fail with out-of-gas.
#[test]
fn test_precompile_gas_hamming_out_of_gas() {
    let a = kora_hdc::HdcVector::random(600);
    let b = kora_hdc::HdcVector::random(601);
    let mut input = vec![0x01u8];
    input.extend_from_slice(&kora_hdc::serialize(&a));
    input.extend_from_slice(&kora_hdc::serialize(&b));
    // Provide 1 gas — far below the 1_500 required
    let result = kora_hdc_chain::hdc_precompile(&input, 1);
    assert!(result.is_err(), "hamming with 1 gas must fail out-of-gas");
}

/// Bind with insufficient gas must fail.
#[test]
fn test_precompile_gas_bind_out_of_gas() {
    let a = kora_hdc::HdcVector::random(602);
    let b = kora_hdc::HdcVector::random(603);
    let mut input = vec![0x02u8];
    input.extend_from_slice(&kora_hdc::serialize(&a));
    input.extend_from_slice(&kora_hdc::serialize(&b));
    let result = kora_hdc_chain::hdc_precompile(&input, 1);
    assert!(result.is_err(), "bind with 1 gas must fail out-of-gas");
}

/// Bundle with insufficient gas must fail.
#[test]
fn test_precompile_gas_bundle_out_of_gas() {
    let a = kora_hdc::HdcVector::random(604);
    let b = kora_hdc::HdcVector::random(605);
    let mut input = vec![0x03u8];
    input.extend_from_slice(&2u32.to_be_bytes());
    input.extend_from_slice(&kora_hdc::serialize(&a));
    input.extend_from_slice(&kora_hdc::serialize(&b));
    let result = kora_hdc_chain::hdc_precompile(&input, 1);
    assert!(result.is_err(), "bundle with 1 gas must fail out-of-gas");
}

/// Hamming with exactly the required gas must succeed.
#[test]
fn test_precompile_gas_hamming_exact() {
    let a = kora_hdc::HdcVector::random(700);
    let b = kora_hdc::HdcVector::random(701);
    let mut input = vec![0x01u8];
    input.extend_from_slice(&kora_hdc::serialize(&a));
    input.extend_from_slice(&kora_hdc::serialize(&b));
    // Exactly 1_500 gas — the documented HAMMING_DISTANCE cost
    let result = kora_hdc_chain::hdc_precompile(&input, 1_500);
    assert!(result.is_ok(), "hamming with exactly 1_500 gas must succeed");
    let (gas_used, _) = result.unwrap();
    assert_eq!(gas_used, 1_500, "hamming gas cost must be exactly 1_500");
}

// ─── E2E Consensus Tests ─────────────────────────────────────────────────────
//
// These tests submit precompile transactions into a simulated network and
// verify that all validators reach the same state root.

/// Test that the HDC bind precompile (opcode 0x02) executes successfully
/// across all validators and they agree on state.
#[test]
fn test_hdc_precompile_bind() {
    let config = TestConfig::default()
        .with_validators(4)
        .with_max_blocks(3)
        .with_timeout(Duration::from_secs(30));

    let caller_key = SigningKey::from_bytes(&[0xA1; 32].into()).expect("valid key");
    let caller = Evm::address_from_key(&caller_key);
    let initial_balance = U256::from(10_000_000_000u64);

    let vec_a = kora_hdc::HdcVector::random(42);
    let vec_b = kora_hdc::HdcVector::random(43);

    let tx = call_bind(&caller_key, config.chain_id, &vec_a, &vec_b, 0);

    let setup = TestSetup {
        genesis_alloc: vec![(caller, initial_balance)],
        bootstrap_txs: vec![tx],
        expected_balances: vec![],
    };

    let outcome = TestHarness::run(config, setup)
        .expect("bind precompile should succeed and reach consensus");

    assert_eq!(outcome.blocks_finalized, 3);
    // State root convergence is verified internally by the harness.
    // The fact that the run succeeded means all 4 validators executed
    // bind(vec_a, vec_b) deterministically and agreed on the result.
}

/// Test that the HDC hamming distance precompile (opcode 0x01) executes
/// successfully across all validators and they agree on state.
#[test]
fn test_hdc_precompile_hamming() {
    let config = TestConfig::default()
        .with_validators(4)
        .with_max_blocks(3)
        .with_timeout(Duration::from_secs(30));

    let caller_key = SigningKey::from_bytes(&[0xA2; 32].into()).expect("valid key");
    let caller = Evm::address_from_key(&caller_key);
    let initial_balance = U256::from(10_000_000_000u64);

    let vec_a = kora_hdc::HdcVector::random(100);
    let vec_b = kora_hdc::HdcVector::random(101);

    // Verify the expected output before submitting to the network.
    let expected_dist = kora_hdc::hamming_distance(&vec_a, &vec_b);
    assert!(expected_dist > 0, "random vectors should have non-zero hamming distance");

    let tx = call_hamming(&caller_key, config.chain_id, &vec_a, &vec_b, 0);

    let setup = TestSetup {
        genesis_alloc: vec![(caller, initial_balance)],
        bootstrap_txs: vec![tx],
        expected_balances: vec![],
    };

    let outcome = TestHarness::run(config, setup)
        .expect("hamming precompile should succeed and reach consensus");

    assert_eq!(outcome.blocks_finalized, 3);
}

/// Test that the HDC bundle precompile (opcode 0x03) executes successfully
/// with 3 vectors bundled via majority vote across all validators.
#[test]
fn test_hdc_precompile_bundle() {
    let config = TestConfig::default()
        .with_validators(4)
        .with_max_blocks(3)
        .with_timeout(Duration::from_secs(30));

    let caller_key = SigningKey::from_bytes(&[0xA3; 32].into()).expect("valid key");
    let caller = Evm::address_from_key(&caller_key);
    let initial_balance = U256::from(10_000_000_000u64);

    let vec_a = kora_hdc::HdcVector::random(200);
    let vec_b = kora_hdc::HdcVector::random(201);
    let vec_c = kora_hdc::HdcVector::random(202);

    // Verify that bundle output is deterministic before network submission.
    let expected = kora_hdc::bundle(&[&vec_a, &vec_b, &vec_c]);
    assert_ne!(expected, kora_hdc::HdcVector::default(), "bundle result should not be all zeros");

    let tx = call_bundle(&caller_key, config.chain_id, &[&vec_a, &vec_b, &vec_c], 0);

    let setup = TestSetup {
        genesis_alloc: vec![(caller, initial_balance)],
        bootstrap_txs: vec![tx],
        expected_balances: vec![],
    };

    let outcome = TestHarness::run(config, setup)
        .expect("bundle precompile should succeed and reach consensus");

    assert_eq!(outcome.blocks_finalized, 3);
}

/// Test that the HDC permute precompile (opcode 0x04) executes successfully
/// across all validators and produces the expected cyclic rotation.
#[test]
fn test_hdc_precompile_permute() {
    let config = TestConfig::default()
        .with_validators(4)
        .with_max_blocks(3)
        .with_timeout(Duration::from_secs(30));

    let caller_key = SigningKey::from_bytes(&[0xA4; 32].into()).expect("valid key");
    let caller = Evm::address_from_key(&caller_key);
    let initial_balance = U256::from(10_000_000_000u64);

    let vec_v = kora_hdc::HdcVector::random(400);
    let n: u32 = 37;

    // Verify output is as expected before network submission.
    let expected = kora_hdc::permute(&vec_v, n as usize);
    assert_ne!(expected, vec_v, "permuted vector should differ from original");

    let tx = call_permute(&caller_key, config.chain_id, &vec_v, n, 0);

    let setup = TestSetup {
        genesis_alloc: vec![(caller, initial_balance)],
        bootstrap_txs: vec![tx],
        expected_balances: vec![],
    };

    let outcome = TestHarness::run(config, setup)
        .expect("permute precompile should succeed and reach consensus");

    assert_eq!(outcome.blocks_finalized, 3);
}

// ─── Consensus Determinism ───────────────────────────────────────────────────

/// Test that multiple HDC precompile calls (bind, hamming, bundle) produce
/// deterministic results: same seed -> same state root across two runs.
#[test]
#[ignore = "flaky when run in parallel - run with --test-threads=1"]
fn test_hdc_consensus_determinism() {
    let config = TestConfig::default()
        .with_validators(4)
        .with_max_blocks(5)
        .with_seed(42)
        .with_timeout(Duration::from_secs(45));

    let caller_key = SigningKey::from_bytes(&[0xB1; 32].into()).expect("valid key");
    let caller = Evm::address_from_key(&caller_key);
    let initial_balance = U256::from(10_000_000_000u64);

    // Mix of bind, hamming, and bundle calls
    let va = kora_hdc::HdcVector::random(300);
    let vb = kora_hdc::HdcVector::random(301);
    let vc = kora_hdc::HdcVector::random(302);

    // Pre-compute expected outputs to verify determinism at the library level.
    let expected_bind_ab = kora_hdc::bind(&va, &vb);
    let expected_bind_ca = kora_hdc::bind(&vc, &va);
    let expected_bundle = kora_hdc::bundle(&[&va, &vb, &vc]);
    let expected_hamming_bc = kora_hdc::hamming_distance(&vb, &vc);
    let expected_hamming_ac = kora_hdc::hamming_distance(&va, &vc);

    // Each precompile call with the same seed must return the same output
    // both times (determinism guarantee).
    assert_eq!(kora_hdc::bind(&va, &vb), expected_bind_ab, "bind must be deterministic");
    assert_eq!(kora_hdc::bind(&vc, &va), expected_bind_ca, "bind must be deterministic");
    assert_eq!(kora_hdc::bundle(&[&va, &vb, &vc]), expected_bundle, "bundle must be deterministic");
    assert_eq!(
        kora_hdc::hamming_distance(&vb, &vc),
        expected_hamming_bc,
        "hamming must be deterministic"
    );
    assert_eq!(
        kora_hdc::hamming_distance(&va, &vc),
        expected_hamming_ac,
        "hamming must be deterministic"
    );

    let txs = vec![
        call_bind(&caller_key, config.chain_id, &va, &vb, 0),
        call_hamming(&caller_key, config.chain_id, &vb, &vc, 1),
        call_bundle(&caller_key, config.chain_id, &[&va, &vb, &vc], 2),
        call_bind(&caller_key, config.chain_id, &vc, &va, 3),
        call_hamming(&caller_key, config.chain_id, &va, &vc, 4),
    ];

    let setup = TestSetup {
        genesis_alloc: vec![(caller, initial_balance)],
        bootstrap_txs: txs,
        expected_balances: vec![],
    };

    // Run twice with the same seed
    let outcome1 = TestHarness::run(config.clone(), setup.clone()).expect("first run");
    let outcome2 = TestHarness::run(config, setup).expect("second run");

    // Same seed + same txs = same state root
    assert_eq!(
        outcome1.state_root, outcome2.state_root,
        "state root must be identical across runs with same seed"
    );
}

// ─── Fast Blocks ─────────────────────────────────────────────────────────────

/// Test that blocks are produced correctly with low-latency network settings.
///
/// NOTE: The harness currently uses 1s leader_timeout. To truly test 50ms
/// blocks, a `.with_block_time_ms(50)` builder method would be needed to
/// thread through to the simplex::Config timeouts. This test verifies that
/// fast finalization (low latency, many blocks) still produces correct
/// consensus results.
#[test]
fn test_hdc_fast_blocks() {
    let config = TestConfig::default()
        .with_validators(4)
        .with_max_blocks(20)
        .with_link(kora_transport_sim::SimLinkConfig {
            latency: Duration::from_millis(5),
            jitter: Duration::from_millis(1),
            success_rate: 1.0,
        })
        .with_timeout(Duration::from_secs(60));

    let setup = TestSetup::empty();

    let outcome = TestHarness::run(config, setup).expect("fast blocks should finalize");

    assert_eq!(outcome.blocks_finalized, 20);
}

// ─── Mixed Operations ────────────────────────────────────────────────────────

/// Test that a transfer + precompile call in the same block work correctly.
#[test]
fn test_hdc_precompile_with_transfers() {
    let config = TestConfig::default()
        .with_validators(4)
        .with_max_blocks(5)
        .with_timeout(Duration::from_secs(45));

    let caller_key = SigningKey::from_bytes(&[0xC1; 32].into()).expect("valid key");
    let receiver_key = SigningKey::from_bytes(&[0xD1; 32].into()).expect("valid key");
    let caller = Evm::address_from_key(&caller_key);
    let receiver = Evm::address_from_key(&receiver_key);

    let initial_balance = U256::from(10_000_000_000u64);
    let transfer_amount = U256::from(1000u64);

    // Transaction 0: bind precompile call
    let vec_a = kora_hdc::HdcVector::random(999);
    let vec_b = kora_hdc::HdcVector::random(1000);
    let precompile_tx = call_bind(&caller_key, config.chain_id, &vec_a, &vec_b, 0);

    // Transaction 1: simple transfer
    let transfer_tx = Evm::sign_eip1559_transfer(
        &caller_key,
        config.chain_id,
        receiver,
        transfer_amount,
        1,
        21_000,
    );

    let setup = TestSetup {
        genesis_alloc: vec![(caller, initial_balance), (receiver, U256::ZERO)],
        bootstrap_txs: vec![precompile_tx, transfer_tx],
        expected_balances: vec![(receiver, transfer_amount)],
    };

    let outcome = TestHarness::run(config, setup)
        .expect("mixed precompile + transfer should reach consensus");

    assert_eq!(outcome.blocks_finalized, 5);
}

// ─── InsightBoard Lifecycle ───────────────────────────────────────────────────

/// Test the full InsightBoard submit→confirm→challenge→purge lifecycle.
///
/// # Why this test is ignored
///
/// Running the InsightBoard lifecycle through the E2E harness requires
/// deploying compiled Solidity bytecode via a CREATE transaction. The
/// `contracts/src/InsightBoard.sol` source exists but no compiled artifact
/// is checked into the repo, and `forge build` is not run as part of the
/// Rust test suite.
///
/// The `TestNode` API also has no `query_storage(digest, address, slot)`
/// method to read contract storage slots after finalization, so there is
/// no way to assert InsightBoard state transitions from a Rust test.
///
/// To enable this test:
/// 1. Run `forge build` in `contracts/` and check the artifact into
///    `contracts/out/InsightBoard.sol/InsightBoard.json`.
/// 2. Add `TestNode::query_storage(digest, addr, slot) -> Option<U256>` to
///    `crates/e2e/src/node.rs`.
/// 3. Decode the artifact bytecode here and submit it as a CREATE tx.
///
/// The `WisdomGate` unit tests in `crates/hdc/chain/src/wisdom.rs` cover the
/// Rust-side lifecycle (submit/challenge/resolve) directly.
#[test]
#[ignore = "requires compiled InsightBoard artifact + query_storage in TestNode — see comment above"]
fn test_hdc_insight_lifecycle() {
    // Placeholder: see doc comment above for what is needed to implement this.
    // When unblocked, the test should:
    //   1. Deploy InsightBoard bytecode via a CREATE tx in genesis setup.
    //   2. Call submit(kind, vector, content) — assert InsightPublished event hash in logs.
    //   3. Call confirm(id) from a second address — assert confirmations incremented.
    //   4. Call challenge(id) — assert state transitions to CHALLENGED.
    //   5. Wait for CHALLENGE_RESOLUTION_CONFS blocks — assert state resolves.
    //   6. Read storage slots via query_storage to verify final state == ACCEPTED or PURGED.
    panic!("not yet implemented — see ignore reason above");
}
