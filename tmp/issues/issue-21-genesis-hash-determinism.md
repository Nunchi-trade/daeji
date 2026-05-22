# Genesis block hash changes on every fresh deployment because timestamp uses wall-clock time

**Complexity: Trivial (one-line fix)**
**Severity: Low**

## Summary

The genesis block hash is non-deterministic across deployments. Every time the devnet is torn down and recreated (e.g., `docker compose down -v && docker compose up`), the `keygen setup` command writes the current wall-clock time into `genesis.json` as the genesis timestamp. Since the genesis block's `timestamp` field is included in its hash computation, every deployment produces a different genesis block hash even though the state root (account allocations) is identical.

This means:
- Block explorers, wallets, and clients cannot hardcode the genesis hash for chain identity verification
- Comparing genesis hashes across validators after a redeployment looks like a consensus failure when it is actually expected
- Any tooling that uses genesis hash as a chain identifier (e.g., EIP-2124 fork identifiers) breaks on redeployment

## Root Cause

### Primary bug: `keygen setup` stamps wall-clock time into genesis.json

In `bin/keygen/src/setup.rs`, lines 190-197, the `GenesisConfig` is constructed with `SystemTime::now()`:

```rust
let genesis = GenesisConfig {
    chain_id: args.chain_id,
    timestamp: std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs(),
    allocations,
};
```

This means every invocation of `keygen setup` writes a different `timestamp` value into `genesis.json`. The current `testnet-artifacts/genesis.json` has `"timestamp": 1778613197`, which is the Unix time at the moment that file was generated.

### Secondary bug: `seed_genesis_block_index` hardcodes `timestamp: 0`

In `crates/node/runner/src/runner.rs`, lines 119-135, `seed_genesis_block_index` hardcodes `timestamp: 0` for the indexed genesis block, regardless of the actual genesis block's timestamp:

```rust
fn seed_genesis_block_index(index: &BlockIndex, genesis: &Block, gas_limit: u64) {
    index.insert_block(
        IndexedBlock {
            hash: genesis.id().0,
            number: 0,
            parent_hash: genesis.parent.0,
            state_root: genesis.state_root.0,
            timestamp: 0,              // <-- hardcoded to 0, ignores genesis.timestamp
            gas_limit,
            gas_used: 0,
            base_fee_per_gas: Some(0),
            transaction_hashes: Vec::new(),
        },
        Vec::new(),
        Vec::new(),
    );
}
```

This creates a timestamp inconsistency: `eth_getBlockByNumber(0)` via RPC reports `timestamp: 0`, but the actual genesis block used for hash computation has a wall-clock timestamp like `1778613197`. The block hash returned by the RPC is computed from the block with the real timestamp, so clients see a hash that does not correspond to a block with `timestamp: 0`. After the primary fix (setting genesis timestamp to 0), these values will naturally align and the inconsistency disappears.

### How the timestamp flows into the block hash

1. `keygen setup` writes timestamp to `genesis.json` (`bin/keygen/src/setup.rs:192`)
2. `BootstrapConfig::load()` reads it (`crates/node/domain/src/bootstrap.rs:63`)
3. Runner passes `bootstrap.genesis_timestamp` to `LedgerView::init_with_genesis_options()` (`crates/node/runner/src/runner.rs:539`)
4. Ledger constructs the genesis `Block` with that timestamp (`crates/node/ledger/src/lib.rs:230`)
5. `Block::id()` hashes the full serialized block including timestamp (`crates/node/domain/src/block.rs`)

## The Fix

### Change: Use timestamp 0 in `keygen setup` (one line)

**File:** `bin/keygen/src/setup.rs`, line 192

**Before:**
```rust
let genesis = GenesisConfig {
    chain_id: args.chain_id,
    timestamp: std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs(),
    allocations,
};
```

**After:**
```rust
let genesis = GenesisConfig {
    chain_id: args.chain_id,
    timestamp: 0,
    allocations,
};
```

This is the simplest possible fix and aligns with:
- `BootstrapConfig::new()` in `crates/node/domain/src/bootstrap.rs:38`, which already defaults `genesis_timestamp` to 0
- `seed_genesis_block_index` in `crates/node/runner/src/runner.rs:126`, which already hardcodes `timestamp: 0` for the indexed block
- Ethereum mainnet convention (genesis block uses timestamp 0)
- The semantic meaning: genesis represents "the beginning of time" for the chain

After this change, the secondary `seed_genesis_block_index` bug becomes moot because the actual genesis timestamp will be 0, matching the hardcoded 0. No code change is needed in the runner, though optionally `timestamp: genesis.timestamp` could be used for correctness.

### Also update: `testnet-artifacts/genesis.json`

The checked-in genesis file currently has the stale wall-clock timestamp:
```json
"timestamp": 1778613197
```

Change to:
```json
"timestamp": 0
```

## Implications for Existing Deployments

- **Existing devnet deployments will need to be redeployed.** After this fix, `keygen setup` will generate a `genesis.json` with `timestamp: 0`, producing a different genesis hash than any prior deployment. Existing nodes that were bootstrapped with a wall-clock timestamp in their genesis will disagree on the genesis hash with newly deployed nodes.
- This is a non-issue in practice because:
  1. The devnet is routinely torn down and redeployed (`just remote-reset`)
  2. There is no persistent mainnet state to preserve
  3. The whole point of the fix is that future deployments produce a stable, reproducible genesis hash

## Affected Files

| File | Line(s) | Change |
|------|---------|--------|
| `bin/keygen/src/setup.rs` | 192-195 | Replace `SystemTime::now()...` with `0` |
| `testnet-artifacts/genesis.json` | 3 | Change `"timestamp": 1778613197` to `"timestamp": 0` |

## Verification

After the fix, every `keygen setup` invocation with the same allocations and chain_id should produce byte-identical `genesis.json` files, and every fresh deployment should produce the same genesis block hash.
