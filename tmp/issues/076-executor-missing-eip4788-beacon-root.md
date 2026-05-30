# 076: Missing EIP-4788 Beacon Root System Contract

**Category:** executor / correctness
**Severity:** low

## Summary

Kora advertises `SpecId::CANCUN` but does not deploy the EIP-4788 beacon block root system contract or perform the required per-block system call. EIP-4788, included in the Cancun hardfork, specifies a system contract at address `0x000F3df6D732807Ef1319fB7B8bB8522d0Beac02` that maintains a ring buffer of recent parent beacon block roots. Contracts querying this address see an empty account instead of the specified data. Since Kora does not have a beacon chain, the omission is architecturally expected, but declaring `SpecId::CANCUN` creates a spec compliance gap.

## Problem

Kora is a minimal Ethereum-compatible execution client that uses Commonware's Simplex BFT for consensus rather than Ethereum's proof-of-stake beacon chain. EIP-4788, required by the Cancun hardfork, has three components -- none of which are implemented:

### 1. No contract deployed at genesis

The genesis allocation format only supports `(Address, U256)` balance pairs and cannot express code or storage deployments:

**File:** `crates/node/domain/src/bootstrap.rs`, lines 12-21

```rust
pub struct BootstrapConfig {
    /// Chain ID declared in the genesis file.
    pub chain_id: u64,
    /// Initial account allocations (address, balance) for genesis.
    pub genesis_alloc: Vec<(Address, U256)>,
    /// Transactions to execute during bootstrap.
    pub bootstrap_txs: Vec<Tx>,
    /// Genesis block Unix timestamp, in seconds.
    pub genesis_timestamp: u64,
}
```

The genesis JSON format mirrors this limitation (`crates/node/domain/src/bootstrap.rs:30-34`):

```rust
struct AllocationJson {
    address: String,
    balance: String,
}
```

There is no way to specify code, nonce, or storage for genesis accounts.

### 2. No per-block system call

The `pre_execute` hook in `BlockExecutor` is a no-op returning an empty changeset:

**File:** `crates/node/executor/src/traits.rs`, lines 22-28

```rust
fn pre_execute(
    &self,
    _context: &BlockContext,
    _state: &S,
) -> Result<ChangeSet, ExecutionError> {
    Ok(ChangeSet::new())
}
```

EIP-4788 requires a system call from `0xfffffffffffffffffffffffffffffffffffffffe` to the beacon root contract with the `parent_beacon_block_root` as calldata before executing user transactions in each block.

### 3. No `parent_beacon_block_root` in block headers

The block header construction in the runner defaults this field to `None` via `..Default::default()`:

**File:** `crates/node/runner/src/runner.rs`, lines 653-660

```rust
let header = Header {
    number: block.height,
    timestamp: block.timestamp,
    gas_limit: self.gas_limit,
    beneficiary: self.fee_recipient,
    base_fee_per_gas: Some(base_fee),
    ..Default::default()
};
```

**File:** `crates/node/runner/src/app.rs`, lines 242-249

```rust
let header = Header {
    number: height,
    timestamp,
    gas_limit: self.gas_limit,
    beneficiary: self.fee_recipient,
    base_fee_per_gas: Some(base_fee),
    ..Default::default()
};
```

Both construction sites omit `parent_beacon_block_root`, which defaults to `None`.

## Code Reference

**File:** `crates/node/executor/src/traits.rs:22-28`
```rust
fn pre_execute(
    &self,
    _context: &BlockContext,
    _state: &S,
) -> Result<ChangeSet, ExecutionError> {
    Ok(ChangeSet::new())
}
```

**File:** `crates/node/domain/src/bootstrap.rs:12-21`
```rust
pub struct BootstrapConfig {
    pub chain_id: u64,
    pub genesis_alloc: Vec<(Address, U256)>,
    pub bootstrap_txs: Vec<Tx>,
    pub genesis_timestamp: u64,
}
```

## Impact

- **Contracts calling the beacon root address see an empty account:** The `CALL` succeeds but returns empty data, causing incorrect behavior for restaking protocols, L2 bridge verifiers, and oracle contracts that expect to read beacon roots from this address.
- **No state divergence within Kora's network:** This is a silent feature omission, not a consensus bug -- all Kora nodes behave identically.
- **Ethereum compatibility gap:** dApps ported from Ethereum mainnet that rely on beacon root queries will not function correctly on Kora.
- **Spec compliance concern:** Declaring `SpecId::CANCUN` implies support for all Cancun EIPs, but EIP-4788 is missing.

## Root Cause

Kora is not a beacon chain client. There is no actual beacon chain producing parent beacon block roots, so there is no natural source for this data. The absence is architecturally expected for an independent EVM chain with its own consensus, but declaring `SpecId::CANCUN` creates a spec compliance gap. Additionally, the genesis format is limited to `(Address, U256)` pairs and cannot deploy contracts with code at genesis.

## Suggested Fix

### 1. Extend the genesis format

```rust
// Before:
pub genesis_alloc: Vec<(Address, U256)>,

// After:
pub struct GenesisAccount {
    pub balance: U256,
    pub nonce: u64,
    pub code: Option<Bytes>,
    pub storage: BTreeMap<U256, U256>,
}
pub genesis_alloc: Vec<(Address, GenesisAccount)>,
```

### 2. Deploy the EIP-4788 contract at genesis

Deploy the canonical EIP-4788 bytecode at address `0x000F3df6D732807Ef1319fB7B8bB8522d0Beac02`.

### 3. Implement the per-block system call

In `RevmExecutor::pre_execute()`, construct a system call from `0xfffffffffffffffffffffffffffffffffffffffe` to the beacon root contract with `parent_beacon_block_root` as calldata, execute against current state, and return the resulting storage writes.

### 4. Set `parent_beacon_block_root` in block headers

In both header construction sites (`runner.rs:653-660` and `app.rs:242-249`), set `parent_beacon_block_root`. Since Kora has no beacon chain, this could be a deterministic derivation from the previous block hash, or `B256::ZERO` as a stub.

## Files to Modify

- `crates/node/executor/src/traits.rs` -- `pre_execute` hook (currently no-op)
- `crates/node/executor/src/revm.rs` -- EVM construction and `BlockEnv` setup
- `crates/node/executor/src/config.rs` -- `SpecId::CANCUN` declaration
- `crates/node/runner/src/runner.rs` (lines 653-660) -- `Header` construction
- `crates/node/runner/src/app.rs` (lines 242-249) -- `Header` construction
- `crates/node/domain/src/bootstrap.rs` (lines 12-21) -- genesis allocation format
- `crates/storage/handlers/src/qmdb.rs` -- `init_genesis()` balance-only account creation

## Related Issues

- `074-executor-cancun-blocks-eip7702.md` -- `SpecId::CANCUN` hardcoded; upgrade to PRAGUE also needs EIP-4788
- `077-executor-eip4844-blob-handling-gaps.md` -- another Cancun EIP with implementation gaps

## Labels

enhancement, correctness, executor
