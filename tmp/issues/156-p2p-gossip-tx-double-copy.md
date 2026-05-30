# P2P Gossip: Unnecessary Full Copy on Both Inbound and Outbound Transaction Paths

**Category**: performance
**Severity**: low

## Summary

Every gossip transaction is copied via `copy_from_slice` on both the outbound and inbound P2P paths, despite `alloy_primitives::Bytes` and `bytes::Bytes` sharing the same underlying `bytes::Bytes` representation. This creates two unnecessary heap allocations per transaction that could be eliminated with zero-copy conversions.

## Problem

Kora's transaction gossip system handles two directions of transaction flow, and both perform an avoidable full-copy of the transaction bytes:

1. **Outbound path** (`crates/node/runner/src/runner.rs`, line 1061): When broadcasting a locally accepted transaction to peers, the code converts `alloy_primitives::Bytes` (the `raw` variable from the internal channel) to `bytes::Bytes` using `copy_from_slice`. This allocates a brand-new heap buffer and copies all bytes, even though `alloy_primitives::Bytes` internally wraps `bytes::Bytes` and a zero-copy conversion exists.

2. **Inbound path** (`crates/node/runner/src/runner.rs`, line 1104): When receiving a gossipped transaction from a peer, the code converts `bytes::Bytes` (the `raw` variable from the P2P layer) to `alloy_primitives::Bytes` using `copy_from_slice`. Again, a full heap copy instead of using the `From<bytes::Bytes>` implementation that `alloy_primitives::Bytes` provides.

Both `alloy_primitives::Bytes` and `bytes::Bytes` are backed by the same `bytes` crate. The `alloy_primitives::Bytes` type is a newtype wrapper around `bytes::Bytes`, and both types implement `From` conversions for zero-copy conversion.

## Code Reference

**Outbound path** -- `crates/node/runner/src/runner.rs:1054-1062`:
```rust
context.child("tx_gossip_out").shared(true).spawn(move |_| async move {
    let mut rx = gossip_outbound_rx;
    while let Some(raw) = rx.recv().await {
        let hash = keccak256(&raw);
        if !mark_seen(&seen, hash) {
            continue;
        }
        let msg = bytes::Bytes::copy_from_slice(&raw);  // <-- UNNECESSARY COPY
        let recipients = sender.send(Recipients::All, msg, false);
```

**Inbound path** -- `crates/node/runner/src/runner.rs:1097-1104`:
```rust
in_metrics.gossip_tx_received.inc();
let hash = keccak256(&raw);
if !mark_seen(&seen, hash) {
    trace!(?hash, ?peer, "tx gossip: skipping already-seen transaction");
    continue;
}

let data = alloy_primitives::Bytes::copy_from_slice(raw.as_ref());  // <-- UNNECESSARY COPY
```

## Impact

At 100+ tx/s gossip rate across a 10-validator network, each transaction creates 2 unnecessary heap allocations (one per direction). This wastes approximately 200 KB/s of allocation bandwidth per node and generates avoidable allocator pressure. The impact scales linearly with both transaction volume and network size, since each peer exchange multiplies the copies. While not a correctness issue, in high-throughput scenarios this adds measurable latency to the gossip pipeline.

## Root Cause

The conversions were implemented using `copy_from_slice` rather than leveraging the `From` trait implementations that both types provide for zero-copy conversion between `alloy_primitives::Bytes` and `bytes::Bytes`.

## Suggested Fix

Replace `copy_from_slice` with zero-copy `From`/`Into` conversions:

**Outbound (line 1061) -- before:**
```rust
let msg = bytes::Bytes::copy_from_slice(&raw);
```
**After:**
```rust
let msg: bytes::Bytes = raw.0.clone();  // Clone the inner bytes::Bytes (ref-counted, no copy)
```
Or, if the `raw` value is not needed afterward:
```rust
let msg: bytes::Bytes = raw.into();
```

**Inbound (line 1104) -- before:**
```rust
let data = alloy_primitives::Bytes::copy_from_slice(raw.as_ref());
```
**After:**
```rust
let data = alloy_primitives::Bytes::from(raw);  // From<bytes::Bytes> impl, zero-copy
```

Verify that `alloy_primitives::Bytes` implements `From<bytes::Bytes>` (it does, since it wraps `bytes::Bytes` internally) and that `bytes::Bytes` can be extracted from `alloy_primitives::Bytes` (via `.0` field access or `Into` impl).

## Files to Modify

- `crates/node/runner/src/runner.rs` -- lines 1061 and 1104

## Related Issues

- `048-p2p-per-tx-state-fetch-gossip-handler.md` -- another performance issue in the gossip handler (per-tx state fetch)
- `159-p2p-gossip-validation-no-batching.md` -- gossip validation creates a new `TransactionValidator` per message

## Labels

`performance`, `p2p`, `good first issue`
