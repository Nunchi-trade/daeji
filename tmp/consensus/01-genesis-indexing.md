# 01 — Genesis Block Indexing

## Problem

`eth_getBlockByNumber("0x0", ...)` returns `null`. The genesis block is never
inserted into `BlockIndex`, so `IndexedStateProvider` has nothing to return.

The `FinalizedReporter` indexes blocks as they finalize through consensus, but
the genesis block is constructed during node initialization — before consensus
starts — and bypasses the reporter pipeline entirely.

## Root Cause

In `crates/node/runner/src/runner.rs`, genesis initialization calls
`init_genesis()` on the storage handler to set up account state, but never
creates a corresponding `IndexedBlock` entry in the `BlockIndex`.

The `BlockIndex` starts empty. Block 1 is the first block that flows through
`FinalizedReporter::report()` → `handle_finalized_update()` →
`block_index.insert_block()`.

## Design

Index the genesis block during node startup, **at the same point where genesis
state is initialized**, using the same `BlockIndex` instance that later gets
passed to `FinalizedReporter` and `IndexedStateProvider`.

This is the right place because:
- The genesis block's fields (state root, timestamp, gas limit, base fee) are
  known at initialization time
- The `BlockIndex` is created before the consensus engine starts
- No race condition: consensus hasn't started yet, so no concurrent writes

### Genesis Block Fields

The genesis `IndexedBlock` should be constructed from the node's genesis
configuration:

```rust
let genesis_block = IndexedBlock {
    hash: genesis_hash,           // Computed from genesis header fields
    number: 0,
    parent_hash: B256::ZERO,      // No parent
    state_root: genesis_state_root, // From init_genesis() result
    timestamp: genesis_timestamp,  // From genesis config (or 0)
    gas_limit: config.gas_limit,
    gas_used: 0,                  // No transactions in genesis
    base_fee_per_gas: Some(initial_base_fee), // See 03-base-fee
    transaction_hashes: vec![],
};

block_index.insert_block(genesis_block, vec![], vec![]);
```

### Where to Put It

In the runner's initialization path, after `init_genesis()` returns the genesis
state root and before the `FinalizedReporter` / `RpcServer` are started:

```
runner.rs init flow:
  1. Create BlockIndex (already happens)
  2. init_genesis() → get state_root  (already happens)
  3. >>> INSERT: build & index genesis block <<<
  4. Create FinalizedReporter with block_index (already happens)
  5. Start RPC server (already happens)
```

### Genesis Hash Computation

The genesis block hash should be computed deterministically from its header
fields, matching what `eth_getBlockByHash` would expect. Use the same header
hashing that the execution layer uses (RLP-encode the header, keccak256).

If the codebase doesn't currently have a standalone header-hash function, add
one to `kora-indexer` or `kora-executor` — it's a utility that will be needed
for `eth_getBlockReceipts` as well.

If adding a proper RLP header hash is too heavy for this workstream, a simpler
approach: compute `keccak256(rlp([parent_hash, ..., base_fee]))` inline.
The genesis hash only needs to be internally consistent — no external chain
needs to agree with it.

## Files to Change

| File | Change |
|------|--------|
| `crates/node/runner/src/runner.rs` | After `init_genesis()`, construct `IndexedBlock` for block 0 and insert into `BlockIndex` |
| `crates/storage/indexer/src/lib.rs` | May need a method to report genesis state root back to caller (if not already returned) |

## Tests

1. **Unit test in `indexed_provider.rs`**: Create a `BlockIndex`, insert a
   genesis block at number 0, verify `block_by_number(0)` returns it with
   correct fields.

2. **Unit test**: Verify `resolve_tag(BlockTag::Earliest)` returns 0, and that
   block 0 is now retrievable (currently the `test_resolve_block_tags` test
   asserts `block.is_none()` for Earliest — update this).

3. **Integration test**: Start a node, query `eth_getBlockByNumber("0x0")`,
   verify non-null response with `number: "0x0"`, `parentHash: 0x00...00`,
   `transactions: []`.

## Verification

```bash
curl -s -X POST $RPC_URL \
  -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","method":"eth_getBlockByNumber","params":["0x0",false],"id":1}' \
  | jq '.result.number'
# Expected: "0x0"  (not null)
```
