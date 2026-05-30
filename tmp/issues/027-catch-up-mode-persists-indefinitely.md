# 027: Catch-Up Mode Can Persist Indefinitely Under Edge Conditions

**Category**: bug
**Severity**: high
**Status**: PARTIALLY MITIGATED
**Labels**: bug, consensus, recovery, reliability

---

## Summary

After a node restart, the node enters catch-up mode and trusts blocks based on finality certificates rather than full re-execution. Catch-up mode exits only when `last_verified_height` advances past `recovered_height + CATCH_UP_THRESHOLD (64)`. While the current code does advance `last_verified_height` through re-encountered certificate-trusted blocks (via the "already verified" early-return path), a theoretical edge case exists where the catch-up window never closes if the tip moves faster than ancestry walks revisit certificate-trusted blocks. In that scenario, the node permanently operates in a degraded mode where it votes on blocks without ever fully verifying state transitions.

---

## Problem

After a restart, the node sets `recovered_height` to the height of the last finalized block and enters catch-up mode. During catch-up, blocks whose parent snapshot is missing are trusted based on their finality certificate (lines 552-574 of `app.rs`). The catch-up window closes when `last_verified_height >= recovered_height + CATCH_UP_THRESHOLD`.

There are three paths that advance `last_verified_height`:

1. **"Already verified" early-return path** (line 490 of `app.rs`): When a block's state root is already in the snapshot store (either from full verification or certificate trust), `last_verified_height` is advanced via `fetch_max`. This path fires when a previously processed block is encountered again during a future ancestry walk.

2. **Full-execution verification** (line 695 of `app.rs`): When a block passes full EVM execution and state root verification, `last_verified_height` is advanced.

3. **Certificate-trust path** (lines 552-574 of `app.rs`): This path explicitly does NOT advance `last_verified_height`, by design. The comment at lines 567-572 explains why.

The mitigation relies on path #1: certificate-trusted blocks insert a state root into the snapshot store via `restore_persisted_snapshot()`, so when those blocks are later encountered in an ancestry walk, the "already verified" check at line 479 succeeds and `last_verified_height` advances.

The edge case: if the network is producing blocks very quickly and the node is far behind, ancestry walks may never revisit the certificate-trusted blocks because the tip keeps advancing. In that scenario, `last_verified_height` never catches up.

**File**: `/Users/will/dev/nunchi/daeji/crates/node/runner/src/app.rs`

---

## Code Reference

The catch-up check (lines 452-467 of `app.rs`):

```rust
// crates/node/runner/src/app.rs:452-467
fn is_catching_up(&self, block_height: u64) -> bool {
    let recovered = self.recovered_height.load(Ordering::Relaxed);
    // Fresh node: never recovered, not catching up.
    if recovered == 0 {
        return false;
    }
    // Block is at or below the recovered height
    if block_height <= recovered {
        return false;
    }
    // Check whether full-execution verification has advanced far enough
    let verified = self.last_verified_height.load(Ordering::Relaxed);
    verified < recovered.saturating_add(CATCH_UP_THRESHOLD)
}
```

The "already verified" early-return path that advances `last_verified_height` (lines 479-496 of `app.rs`):

```rust
// crates/node/runner/src/app.rs:479-496
if self.ledger.query_state_root(digest).await.is_some() {
    // Block is already in the snapshot store. Advance last_verified_height
    // so the catch-up window eventually closes.
    self.last_verified_height.fetch_max(block.height, Ordering::Relaxed);
    if let Some(ref state) = self.node_state {
        state.set_last_verified_height(block.height);
    }
    trace!(?digest, height = block.height, "block already verified");
    return true;
}
```

The certificate-trust path that does NOT advance `last_verified_height` (lines 552-574 of `app.rs`):

```rust
// crates/node/runner/src/app.rs:552-574
if self.is_catching_up(block.height) {
    debug!(
        ?digest,
        ?parent_digest,
        height = block.height,
        recovered_height = self.recovered_height.load(Ordering::Relaxed),
        last_verified = self.last_verified_height.load(Ordering::Relaxed),
        "verify_block: parent snapshot missing during catch-up; \
         trusting finality certificate"
    );
    self.ledger.restore_persisted_snapshot(block).await;
    // We do NOT update last_verified_height here because
    // certificate-trust is not full verification.  However,
    // the "already verified" early-return path at the top of
    // verify_block WILL advance last_verified_height when
    // this block is encountered again in a future ancestry
    // walk, ensuring the catch-up window eventually closes.
    return true;
}
```

The CATCH_UP_THRESHOLD constant (line 86 of `app.rs`):

```rust
// crates/node/runner/src/app.rs:86
const CATCH_UP_THRESHOLD: u64 = 64;
```

The full-execution path that advances `last_verified_height` (line 695 of `app.rs`):

```rust
// crates/node/runner/src/app.rs:695
let prev_verified = self.last_verified_height.fetch_max(block.height, Ordering::Relaxed);
```

---

## Impact

A node stuck in catch-up mode indefinitely exhibits these behaviors:

1. **No state verification**: The node votes to approve blocks based solely on their finality certificate, never actually executing the transactions or verifying the state root. It trusts that 2/3+ of other validators verified correctly.
2. **Operationally invisible**: The node appears healthy in all standard metrics (block height advances, peer connections are maintained, votes are cast). There is no metric or log that indicates the node is permanently in catch-up mode.
3. **Reduced security**: If multiple nodes are stuck in catch-up mode simultaneously, the effective security threshold is reduced because fewer nodes are performing full verification.

The condition is more likely when:
- The network produces many blocks during the restart window (large gap to catch up).
- The node's QMDB state is far behind the tip.
- Memory pressure causes snapshot eviction before catch-up completes.

---

## Root Cause

The catch-up exit condition requires `last_verified_height` to advance past `recovered_height + 64`. The primary mechanism for this advancement during catch-up is the "already verified" early-return path at line 479-490, which fires when previously processed blocks are re-encountered in ancestry walks. If the tip advances faster than blocks are re-encountered, the threshold is never reached.

---

## Suggested Fix

1. **Add a secondary exit condition based on elapsed blocks**: If the node has been running for more than `MAX_CATCH_UP_WINDOW` blocks since restart, exit catch-up mode regardless of `last_verified_height`:

```rust
const MAX_CATCH_UP_WINDOW: u64 = 256;

fn is_catching_up(&self, block_height: u64) -> bool {
    let recovered = self.recovered_height.load(Ordering::Relaxed);
    if recovered == 0 {
        return false;
    }
    if block_height <= recovered {
        return false;
    }
    let verified = self.last_verified_height.load(Ordering::Relaxed);
    let elapsed = block_height.saturating_sub(recovered);
    // Exit if either: verified enough blocks, or ran for long enough
    verified < recovered.saturating_add(CATCH_UP_THRESHOLD)
        && elapsed < MAX_CATCH_UP_WINDOW
}
```

2. **Add a metric for catch-up status**: Expose a `kora_catching_up` boolean gauge metric so operators can detect when catch-up mode is active and alert if it persists:

```rust
if self.is_catching_up(block.height) {
    if let Some(ref m) = self.metrics {
        m.catching_up.set(1);
    }
} else {
    if let Some(ref m) = self.metrics {
        m.catching_up.set(0);
    }
}
```

3. **Force state resync**: If catch-up mode has persisted for more than 1024 blocks, log an error and consider triggering a full state resync from peers.

---

## Files to Modify

- `/Users/will/dev/nunchi/daeji/crates/node/runner/src/app.rs` -- Add `MAX_CATCH_UP_WINDOW` constant and secondary exit condition to `is_catching_up()` (line 452). Add catch-up metric.

---

## Related Issues

- `029-snapshot-wait-timeout-nullifications.md` (snapshot availability affects catch-up)
