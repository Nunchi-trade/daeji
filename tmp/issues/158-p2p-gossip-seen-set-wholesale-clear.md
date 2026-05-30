# P2P Gossip: Seen-Set Wholesale Clear Causes Re-Broadcast Storm

**Category**: performance
**Severity**: medium

## Summary

The gossip deduplication seen-set uses a `HashSet` that is cleared wholesale when it reaches 65,536 entries. After clearing, the node loses memory of all recently-seen transactions, causing it to re-validate and attempt to re-broadcast transactions it already processed. Under sustained load this creates periodic CPU and bandwidth spikes every time the set wraps around.

## Problem

Kora's transaction gossip deduplication relies on a bounded `HashSet` of transaction hashes called the "seen-set." When a transaction hash is encountered (either inbound from peers or outbound from local submission), it is inserted into this set. If the hash is already present, the transaction is skipped as a duplicate.

The problem is the eviction strategy: when the set reaches `TX_GOSSIP_SEEN_SET_CAPACITY` (65,536 entries), the entire set is cleared with `set.clear()`. This means:

1. Immediately after clearing, the node has zero memory of any previously-seen transactions.
2. If peers re-gossip transactions that the node already has in its mempool, the node will re-validate them (running ECDSA signature recovery, state lookups for balance/nonce checks, and pool insertion logic).
3. On the outbound side, the node may attempt to re-broadcast transactions it already sent.
4. The txpool ultimately rejects duplicates (returning `AlreadyExists` / `NonceAlreadyInPool`), so this is not a correctness issue, but the expensive validation path is executed unnecessarily.

Under a sustained load test sending 65K+ unique transactions, this creates periodic CPU/bandwidth spikes every time the set wraps around. On a 10-validator network, each clear event triggers a "thundering herd" effect where multiple nodes simultaneously re-validate the same pool of transactions.

## Code Reference

**Seen-set type and capacity** -- `crates/node/runner/src/runner.rs:82-84`:
```rust
/// Maximum number of transaction hashes retained in the gossip seen-set.
/// When the set exceeds this size it is cleared to avoid unbounded memory
/// growth. Under normal load the TTL-based cleanup keeps the set far smaller.
const TX_GOSSIP_SEEN_SET_CAPACITY: usize = 65_536;
```

**Seen-set type alias** -- `crates/node/runner/src/runner.rs:713`:
```rust
type SeenSet = Arc<parking_lot::Mutex<HashSet<B256>>>;
```

**`mark_seen` function with wholesale clear** -- `crates/node/runner/src/runner.rs:720-727`:
```rust
/// Returns `true` if the hash was **not** previously present (i.e. it is new).
fn mark_seen(seen: &SeenSet, hash: B256) -> bool {
    let mut set = seen.lock();
    if set.len() >= TX_GOSSIP_SEEN_SET_CAPACITY {
        debug!(capacity = TX_GOSSIP_SEEN_SET_CAPACITY, "tx gossip seen-set full, clearing");
        set.clear();
    }
    set.insert(hash)
}
```

The `mark_seen` function is called from three locations:
- Outbound gossip (line 1058): before broadcasting a locally accepted transaction
- Inbound gossip (line 1099): before validating a peer's transaction
- RPC submission (line 1224): before forwarding a user-submitted transaction to gossip

## Impact

- **Periodic CPU spikes**: Every 65,536 unique transactions, all recent dedup knowledge is lost. In a burst scenario, the node may re-validate thousands of transactions that are already in the mempool.
- **Wasted bandwidth**: Re-broadcast of transactions the node already sent to peers.
- **Amplified by network size**: On a 10-node network, if multiple nodes clear their seen-sets around the same time, they create cascading re-gossip waves.
- **Not a correctness bug**: The txpool itself performs its own deduplication check, so no transaction is actually inserted twice. The issue is purely wasted CPU and bandwidth.

## Root Cause

The seen-set uses a simple `HashSet<B256>` with a wholesale `clear()` eviction strategy. This was chosen for simplicity (as noted in the code comment: "this is cheaper than an LRU and perfectly safe because the txpool itself provides the ultimate dedup"). However, the "safe" claim only covers correctness, not performance.

## Suggested Fix

Replace the `HashSet` + wholesale clear with a bounded data structure that evicts old entries incrementally:

**Option A -- LRU cache** (recommended for simplicity):
```rust
use lru::LruCache;
use std::num::NonZeroUsize;

type SeenSet = Arc<parking_lot::Mutex<LruCache<B256, ()>>>;

fn new_seen_set() -> SeenSet {
    Arc::new(parking_lot::Mutex::new(
        LruCache::new(NonZeroUsize::new(TX_GOSSIP_SEEN_SET_CAPACITY).unwrap())
    ))
}

fn mark_seen(seen: &SeenSet, hash: B256) -> bool {
    let mut cache = seen.lock();
    if cache.contains(&hash) {
        false  // already seen
    } else {
        cache.put(hash, ());  // evicts oldest entry if at capacity
        true  // new
    }
}
```

**Option B -- Dual bloom filters**: Maintain two counting Bloom filters, rotating on a timer (e.g., every 30 seconds). This gives constant-memory, constant-time deduplication with no spikes, at the cost of a small false-positive rate (transactions incorrectly marked as "seen").

**Option C -- Ring buffer**: A fixed-size circular buffer of hashes that overwrites the oldest entry on insertion. Simple and cache-friendly.

Any of these approaches eliminates the periodic spike while maintaining the same memory bound.

## Files to Modify

- `crates/node/runner/src/runner.rs` -- replace `SeenSet` type, `new_seen_set()`, and `mark_seen()` functions (lines 713-727)

## Related Issues

- `016-seed-tracker-unbounded.md` -- another unbounded data structure concern
- `161-p2p-gossip-no-backpressure.md` -- gossip handler also lacks backpressure, compounding the re-validation load after a clear

## Labels

`performance`, `p2p`
