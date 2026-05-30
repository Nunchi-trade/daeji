# 072: State Root Is a Keccak Hash Chain, Not a Merkle Patricia Trie -- No eth_getProof Possible

**Category:** storage / correctness
**Severity:** medium

## Summary

Kora computes its "state root" as a keccak256 hash chain over changesets rather than using a Merkle Patricia Trie (MPT). This produces a valid deterministic commitment (all validators agree on the same root, sufficient for consensus), but it is structurally impossible to extract Merkle proofs from the result. The `eth_getProof` RPC method cannot be implemented, and any protocol requiring Merkle state proofs (light clients, cross-chain bridges, fault proofs) will not work with Kora.

## Problem

Kora is a minimal Ethereum-compatible execution client that uses QMDB (a three-partition key-value store) for state storage instead of the standard Merkle Patricia Trie used by geth and reth. The state root is computed in two ways, both of which are simple hash chains:

**File:** `crates/storage/qmdb/src/root.rs`, lines 14-65

1. `StateRoot::compute()` (line 16) combines three partition roots by concatenation and hashing:
   ```
   keccak256(namespace || accounts_root || storage_root || code_root)
   ```

2. `StateRoot::transition()` (line 26) computes a new root from a parent root and a changeset:
   ```
   keccak256(namespace || parent_root || changeset_bytes)
   ```

There are no intermediate trie nodes, no branching structure, and no way to produce a proof for any individual account without replaying the entire chain of transitions.

The `stateRoot` field in RPC block responses (served from `crates/node/rpc/src/indexed_provider.rs`) returns this hash chain value, which looks like a standard 32-byte state root but is not MPT-compatible.

The `eth_getProof` method is completely absent from the codebase (neither defined in the `EthApi` trait in `crates/node/rpc/src/eth.rs` nor mentioned anywhere).

## Code Reference

**File:** `crates/storage/qmdb/src/root.rs:14-65`
```rust
impl StateRoot {
    /// Compute state root from three partition roots.
    pub fn compute(accounts_root: B256, storage_root: B256, code_root: B256) -> B256 {
        let mut buf = Vec::with_capacity(KORA_ROOT_NAMESPACE.len() + 96);
        buf.extend_from_slice(KORA_ROOT_NAMESPACE);
        buf.extend_from_slice(accounts_root.as_slice());
        buf.extend_from_slice(storage_root.as_slice());
        buf.extend_from_slice(code_root.as_slice());
        keccak256(buf)
    }

    /// Compute a deterministic consensus root from a parent root and state transition.
    pub fn transition(parent_root: B256, changes: &ChangeSet) -> B256 {
        if changes.is_empty() {
            return parent_root;
        }

        let estimated =
            KORA_TRANSITION_ROOT_NAMESPACE.len() + 32 + 8 + changes.accounts.len() * 128;
        let mut buf = Vec::with_capacity(estimated);
        buf.extend_from_slice(KORA_TRANSITION_ROOT_NAMESPACE);
        buf.extend_from_slice(parent_root.as_slice());
        buf.extend_from_slice(&(changes.accounts.len() as u64).to_be_bytes());

        for (address, update) in &changes.accounts {
            buf.extend_from_slice(address.as_slice());
            buf.push(u8::from(update.created));
            buf.push(u8::from(update.selfdestructed));
            buf.extend_from_slice(&update.nonce.to_be_bytes());
            buf.extend_from_slice(&update.balance.to_be_bytes::<32>());
            buf.extend_from_slice(update.code_hash.as_slice());
            // ... code and storage serialization continues ...
        }

        keccak256(buf)
    }
}
```

## Impact

1. **`eth_getProof` cannot be implemented** -- there is no trie structure from which to extract account or storage proofs. Any tool or protocol that calls `eth_getProof` will get a "method not found" error.

2. **Light client verification is impossible** -- light clients that verify state via Merkle proofs against the state root cannot work with Kora's hash chain.

3. **Cross-chain bridges cannot verify Kora state** -- bridge protocols (e.g., IBC, LayerZero, Axelar) that prove on-chain state require Merkle proofs from the state root.

4. **On-chain fault proofs are impossible** -- protocols like Optimism's dispute game that re-execute state transitions against Merkle proofs cannot use Kora's state root.

5. **The `stateRoot` field is misleading** -- the RPC returns a 32-byte hash that looks like a standard state root. Tools that assume it is MPT-compatible will fail silently when attempting to verify proofs against it.

## Root Cause

Architectural decision: QMDB uses a three-partition layout (accounts, storage, code) with a hash chain commitment scheme rather than an MPT. This was likely chosen for performance -- MPT updates are I/O-intensive (O(log n) trie node touches per account update), which is the primary bottleneck in geth and reth. The hash chain approach is O(1) per changeset, enabling Kora's high throughput (33+ blocks/s).

## Suggested Fix

**Option A (Full MPT):** Replace the hash chain with a proper MPT using `alloy-trie` or `eth-trie`. This requires maintaining a trie structure alongside QMDB, re-computing trie nodes on every state transition, and storing trie nodes for proof generation. Provides full Ethereum spec compliance but is a significant refactor with potential throughput regression.

**Option B (Document deviation):** Add `eth_getProof` as an RPC method that returns a clear error (`-32601`, "Kora uses a non-MPT state commitment scheme"). Document in operator guides that `stateRoot` is a consensus commitment hash, not an MPT root. This is the pragmatic approach if proof serving is not a near-term requirement.

**Option C (Hybrid):** Maintain the fast hash chain for consensus and build an MPT asynchronously in the background for proof serving. The MPT would lag behind head by a few blocks but could serve `eth_getProof` for finalized state. Preserves throughput but adds dual storage overhead.

## Files to Modify

- `crates/storage/qmdb/src/root.rs` (lines 14-65) -- `StateRoot::compute()` and `StateRoot::transition()` implementations
- `crates/node/rpc/src/indexed_provider.rs` -- `stateRoot` field in RPC responses
- `crates/node/rpc/src/eth.rs` -- add `eth_getProof` method (either implementation or explicit error)

## Related Issues

- `078-rpc-missing-standard-methods.md` -- `eth_getProof` is one of many missing RPC methods

## Labels

enhancement, correctness, storage, documentation
