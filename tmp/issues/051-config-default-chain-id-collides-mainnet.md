# Default Chain ID (1) Collides With Ethereum Mainnet

**Category:** config, security
**Severity:** medium

## Summary

The Kora binary's default chain ID is `1`, which is the Ethereum mainnet chain ID. If an operator starts a Kora validator without explicitly setting a chain ID (e.g., omits `--chain-id` and has no config file), the node creates a chain whose EIP-155 replay protection domain matches Ethereum mainnet. This means transactions signed for Kora could be replayed on Ethereum mainnet, and vice versa, if the same account keys are used on both chains.

## Problem

Kora is an EVM-compatible execution client built on commonware simplex BFT consensus. Its node configuration defines a default chain ID constant at build time, used when no CLI flag or config file overrides it.

In `crates/node/config/src/node.rs`, line 11, the default chain ID is set to `1`:

```rust
pub const DEFAULT_CHAIN_ID: u64 = 1;
```

This constant is used by the `NodeConfig::default()` implementation (line 57) and by serde deserialization of config files that omit the `chain_id` field (line 23-24, via the `default_chain_id()` function at line 199).

The Docker compose files (`docker/compose/devnet.yaml`) override this to `1337` via the `CHAIN_ID` environment variable (line 35: `CHAIN_ID=${CHAIN_ID:-1337}`), so the Docker-based devnet is not affected. However, any operator running the binary directly without a config file will get chain ID `1`.

## Code Reference

File: `crates/node/config/src/node.rs`, lines 11 and 199-201:

```rust
/// Default chain ID for local development.
pub const DEFAULT_CHAIN_ID: u64 = 1;

// ...

const fn default_chain_id() -> u64 {
    DEFAULT_CHAIN_ID
}
```

File: `crates/node/config/src/node.rs`, lines 21-24 (where the default is applied during config deserialization):

```rust
pub struct NodeConfig {
    /// Chain ID for the network.
    #[serde(default = "default_chain_id")]
    pub chain_id: u64,
```

## Impact

1. **Cross-chain transaction replay**: EIP-155 replay protection encodes the chain ID into the transaction signature. When two chains share the same chain ID (`1`), a transaction valid on one chain is also valid on the other. If a user or operator uses the same private key on both Kora and Ethereum mainnet, any transaction sent on Kora can be extracted from its mempool or block and submitted to Ethereum mainnet (and vice versa). This is a concrete attack vector when chain IDs match.

2. **Tooling confusion**: Wallets (MetaMask, etc.), block explorers, and RPC clients check the chain ID to determine which network they are connected to. Chain ID `1` tells all standard tooling that this is Ethereum mainnet. This can cause MetaMask to display incorrect network names, block explorers to misidentify the chain, and automated tools to treat testnet tokens as mainnet ETH.

3. **Operational risk for new operators**: Someone following a "quickstart" guide who runs `kora validator --data-dir /data --peers peers.json` without `--chain-id` will unknowingly create an Ethereum-mainnet-domain chain.

## Root Cause

The default was set to `1` during early development, likely for compatibility with Ethereum tooling that defaults to mainnet. It was never updated to a private-use chain ID for the production binary.

## Suggested Fix

Change the default to a chain ID from the private-use range:

**Before** (`crates/node/config/src/node.rs:11`):
```rust
pub const DEFAULT_CHAIN_ID: u64 = 1;
```

**After** (option A -- align with Docker devnet):
```rust
pub const DEFAULT_CHAIN_ID: u64 = 1337;  // Standard devnet chain ID
```

**After** (option B -- unique project identifier):
```rust
pub const DEFAULT_CHAIN_ID: u64 = 0x4B6F7261;  // "Kora" in ASCII hex (1266532450)
```

Option A is preferred because it aligns the binary default with the Docker compose configuration (`CHAIN_ID=${CHAIN_ID:-1337}`), reducing the delta between "run directly" and "run via Docker". The existing test `test_default_config` (line 218) would need its assertion updated accordingly.

## Files to Modify

- `crates/node/config/src/node.rs` -- change `DEFAULT_CHAIN_ID` constant on line 11

## Related Issues

- `051-config-default-chain-id-collides-mainnet.md` (this file)
- `055-docker-no-config-toml-in-entrypoint.md` -- the entrypoint does not generate a config file, so chain ID is only set via CLI; if the CLI flag were ever dropped, the binary default would take effect

## Labels

bug, security, config
