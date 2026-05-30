# Loadgen Funded Accounts Use Trivially Guessable Private Keys

**Category**: security
**Severity**: medium
**Labels**: `security`, `config`, `bug`

## Summary

The `loadgen_address()` function in the devnet setup tool generates secp256k1 private keys from seeds where `secret[31] = seed` and all other bytes are zero, producing private keys like `0x0000...0001` through `0x0000...0032` (hex for 1 through 50). These are the most well-known private keys in the Ethereum ecosystem -- they are actively monitored by sweeper bots on all public networks. Each of these addresses is funded with 1,000,000 ETH in the genesis allocation. If this genesis configuration is accidentally used on a semi-public or production network, all funds would be drained within seconds.

## Problem

In `bin/keygen/src/setup.rs`, the `loadgen_address()` function at lines 72-81 creates deterministic private keys from trivially guessable seeds:

```rust
fn loadgen_address(seed: u8) -> Address {
    let mut secret = [0u8; 32];
    secret[31] = seed;  // private key = 0x0000...00{seed} (e.g., 0x01, 0x02, ... 0x32)
    let key = SigningKey::from_bytes((&secret).into())
        .expect("loadgen seed should produce valid secp256k1 key");
    let encoded = key.verifying_key().to_encoded_point(false);
    let pubkey = encoded.as_bytes();
    let hash = keccak256(&pubkey[1..]);
    Address::from_slice(&hash[12..])
}
```

This function is called by `funded_loadgen_allocations()` at lines 83-85 to generate 50 funded accounts (seeds 1 through 50):

```rust
fn funded_loadgen_allocations() -> impl Iterator<Item = GenesisAllocation> {
    (1..=LOADGEN_ACCOUNT_COUNT).map(|seed| funded_allocation(loadgen_address(seed).to_string()))
}
```

Each allocation receives `GENESIS_BALANCE = "1000000000000000000000000"` (1 million ETH) as defined at line 15.

The resulting genesis allocations are written to `genesis.json` at lines 188-201, which is loaded at validator startup. The test fixtures at lines 234-238 confirm these are indeed the well-known addresses:

```rust
const LOADGEN_ADDRESS_FIXTURES: &[(u8, &str)] = &[
    (1, "0x7E5F4552091A69125d5DfCb7b8C2659029395Bdf"),  // private key = 0x01
    (2, "0x2B5AD5c4795c026514f8317c7a215E218DcCD6cF"),  // private key = 0x02
    (3, "0x6813Eb9362372EEF6200f3b1dbC3f819671cBA69"),  // private key = 0x03
];
```

These addresses (particularly private key = 1, 2, 3) are among the most famous in Ethereum and are listed in every "do not use" guide.

## Code Reference

`loadgen_address()` in `/Users/will/dev/nunchi/daeji/bin/keygen/src/setup.rs` (lines 72-81):

```rust
fn loadgen_address(seed: u8) -> Address {
    let mut secret = [0u8; 32];
    secret[31] = seed;
    let key = SigningKey::from_bytes((&secret).into())
        .expect("loadgen seed should produce valid secp256k1 key");
    let encoded = key.verifying_key().to_encoded_point(false);
    let pubkey = encoded.as_bytes();
    let hash = keccak256(&pubkey[1..]);
    Address::from_slice(&hash[12..])
}
```

`funded_loadgen_allocations()` in the same file (lines 83-85):

```rust
fn funded_loadgen_allocations() -> impl Iterator<Item = GenesisAllocation> {
    (1..=LOADGEN_ACCOUNT_COUNT).map(|seed| funded_allocation(loadgen_address(seed).to_string()))
}
```

Constants in the same file (lines 15-16):

```rust
const GENESIS_BALANCE: &str = "1000000000000000000000000";
const LOADGEN_ACCOUNT_COUNT: u8 = 50;
```

Usage in `run()` at lines 188-197:

```rust
    let mut allocations = vec![
        funded_allocation("0x0000000000000000000000000000000000000001"),
        funded_allocation("0xEb1Ba7Fc58b3416361a0EE07d140c91410c0AA8c"),
        // ... more hardcoded allocations ...
    ];
    allocations.extend(funded_loadgen_allocations());
```

## Impact

**Loss of genesis funds if devnet genesis is reused in a non-test context.** Specific scenarios:

1. **Semi-public testnet**: An operator reuses the devnet `genesis.json` for a public testnet. Sweeper bots monitoring private keys 1-50 will drain all 50 million ETH (50 accounts x 1M ETH) within seconds of the first block.

2. **Production accident**: A genesis.json intended for production is accidentally generated with the loadgen allocations included. Even though Kora is not targeting Ethereum mainnet, any EVM network with real economic value is vulnerable.

3. **Key reuse across chains**: Private keys 1-50 are well-known across all EVM chains. Any cross-chain bridge or tool that recognizes these addresses may flag the chain as compromised.

4. **No runtime guard**: The code has no check to prevent these insecure allocations from being generated when `chain_id = 1` (mainnet) or any non-test chain ID.

## Root Cause

The function prioritizes simplicity and determinism for load testing: using sequential integers as private keys makes it trivial for the load generator to derive the same keys at runtime (see `bin/loadgen/src/main.rs`). There is no warning log, no guard against non-devnet usage, and no documentation marking these as insecure.

## Suggested Fix

**Option 1 (recommended): Add a prominent warning log and chain_id guard.**

```rust
fn loadgen_address(seed: u8, chain_id: u64) -> Address {
    if chain_id == 1 {
        panic!("REFUSING to generate loadgen addresses for chain_id 1 (Ethereum mainnet)");
    }
    tracing::warn!(
        seed,
        chain_id,
        "Generating loadgen account with INSECURE deterministic key (devnet only!)"
    );
    let mut secret = [0u8; 32];
    secret[31] = seed;
    // ...
}
```

**Option 2: Derive keys from chain_id to avoid cross-chain reuse.**

```rust
fn loadgen_address(seed: u8, chain_id: u64) -> Address {
    let mut hasher = keccak256;
    let mut input = Vec::new();
    input.extend_from_slice(b"kora-loadgen-v1");
    input.extend_from_slice(&chain_id.to_le_bytes());
    input.push(seed);
    let secret = keccak256(&input);
    let key = SigningKey::from_bytes((&secret).into()).expect("...");
    // ...
}
```

This produces unique, non-guessable keys per chain while remaining deterministic for the load generator.

**Option 3 (minimal): Add a warning log at genesis generation time.**

At `/Users/will/dev/nunchi/daeji/bin/keygen/src/setup.rs:197`:

```rust
tracing::warn!(
    "Genesis includes {} accounts with INSECURE loadgen keys (private key = 1..{}). \
     These keys are publicly known. DO NOT use this genesis for production.",
    LOADGEN_ACCOUNT_COUNT, LOADGEN_ACCOUNT_COUNT
);
allocations.extend(funded_loadgen_allocations());
```

## Files to Modify

- `/Users/will/dev/nunchi/daeji/bin/keygen/src/setup.rs` -- `loadgen_address()` at line 72, `funded_loadgen_allocations()` at line 83, and `run()` at line 197

## Related Issues

- Local file `051-config-default-chain-id-collides-mainnet.md` -- Default chain_id = 1 collides with Ethereum mainnet (compounds this issue: a fresh default setup generates insecure keys with the mainnet chain_id)
