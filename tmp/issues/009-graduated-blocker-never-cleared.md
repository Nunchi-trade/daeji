# 009: GraduatedBlocker Catch-Up Flag Never Cleared -- Byzantine Peers Cannot Be Banned After Restart

**Category:** bug / consensus / networking / recovery
**Severity:** high
**Labels:** bug, security, consensus, p2p, recovery

---

## Summary

The `GraduatedBlocker` wraps the P2P oracle's `block()` method to suppress peer bans during catch-up. When a node restarts and detects existing archived data, it sets the `catching_up` flag to `true`. This flag is **never set back to `false`** for the entire lifetime of the node process. After any restart, the node permanently suppresses peer blocking for the P2P resolver, meaning Byzantine peers can never be banned regardless of their behavior.

## Problem

The `GraduatedBlocker` at `crates/node/runner/src/runner.rs:95-145` wraps the P2P oracle's `Blocker` trait. When the `catching_up` flag is `true`, all `block()` calls are suppressed (returning `Feedback::Ok` instead of banning the peer). The flag is initialized at line 1348 based on whether the node is recovering from a restart:

```rust
let resolver_catching_up = Arc::new(AtomicBool::new(recovered_head_height.is_some()));
```

The flag is set to `true` when `recovered_head_height.is_some()` (i.e., whenever the node restarts with existing archived data). However, **no code ever sets it back to `false`**.

The code at lines 114-116 explicitly acknowledges this limitation:

> "A future improvement should wire a 'backfill complete' signal from the resolver to clear this flag once historical block sync finishes."

Separately, the `RevmApplication::is_catching_up()` method at `crates/node/runner/src/app.rs:452-467` does track when catch-up is complete (when `last_verified_height >= recovered_height + CATCH_UP_THRESHOLD`), but this is a **different mechanism** that does not share state with the `GraduatedBlocker`. The catch-up completion detection at lines 699-706 logs a message but does not clear the `GraduatedBlocker` flag:

```rust
if prev_verified < self.recovered_height.load(Ordering::Relaxed)
    && block.height >= self.recovered_height.load(Ordering::Relaxed)
{
    info!(
        height = block.height,
        recovered_height = self.recovered_height.load(Ordering::Relaxed),
        "catch-up: first full-execution verification past recovery point"
    );
    // NOTE: does NOT clear the GraduatedBlocker's catching_up flag
}
```

## Code Reference

**GraduatedBlocker definition -- `crates/node/runner/src/runner.rs:95-145`:**

```rust
/// A [`Blocker`] that suppresses peer bans during catch-up but delegates to
/// the real oracle blocker during normal operation.
///
/// ...
///
/// The `catching_up` flag is set to `true` when the node is recovering from a
/// restart (i.e., `recovered_head_height` is `Some`) and cleared to `false`
/// for fresh genesis starts. A future improvement should wire a "backfill
/// complete" signal from the resolver to clear this flag once historical block
/// sync finishes.
#[derive(Clone, Debug)]
struct GraduatedBlocker<P: commonware_cryptography::PublicKey> {
    oracle: commonware_p2p::authenticated::discovery::Oracle<P>,
    catching_up: Arc<AtomicBool>,  // Set to true on restart, NEVER set to false
}

impl<P: commonware_cryptography::PublicKey> Blocker for GraduatedBlocker<P> {
    type PublicKey = P;

    fn block(&mut self, peer: Self::PublicKey) -> Feedback {
        let catching_up = self.catching_up.load(Ordering::Relaxed);
        if catching_up {
            warn!(?peer, "GraduatedBlocker: suppressing block request during catch-up");
            Feedback::Ok  // Byzantine peer NOT banned
        } else {
            warn!(?peer, "GraduatedBlocker: blocking Byzantine peer via oracle");
            self.oracle.block(peer)
        }
    }
}
```

**Flag initialization -- `crates/node/runner/src/runner.rs:1348`:**

```rust
let resolver_catching_up = Arc::new(AtomicBool::new(recovered_head_height.is_some()));
let resolver_blocker =
    GraduatedBlocker::new(transport.oracle.clone(), resolver_catching_up);
```

**Catch-up completion detection (does NOT clear flag) -- `crates/node/runner/src/app.rs:699-706`:**

```rust
if prev_verified < self.recovered_height.load(Ordering::Relaxed)
    && block.height >= self.recovered_height.load(Ordering::Relaxed)
{
    info!(
        height = block.height,
        recovered_height = self.recovered_height.load(Ordering::Relaxed),
        "catch-up: first full-execution verification past recovery point"
    );
}
```

## Impact

After any restart (which happens frequently due to OOM kills on the devnet), the node permanently suppresses peer blocking for the P2P resolver. A Byzantine peer can:

1. **Continuously send invalid backfill data** without ever being banned, wasting the node's bandwidth and CPU processing invalid blocks.
2. **Slow down catch-up** by flooding with garbage data that must be parsed, verified, and discarded.
3. **Prevent effective peer management**: The resolver's built-in Byzantine fault detection is completely disabled, meaning misbehaving peers face no consequences.

The impact is amplified in long-running networks where nodes restart occasionally (due to OOM kills, upgrades, or maintenance). Each restarted node becomes permanently vulnerable to Byzantine peer abuse until the node process is fully stopped and restarted from genesis (not just a Docker restart).

## Root Cause

The `catching_up` flag was designed as a one-way switch (false-to-true on restart) without implementing the reverse transition. The `RevmApplication` tracks catch-up completion internally (via `last_verified_height` vs `recovered_height + CATCH_UP_THRESHOLD`) but does not share this state with the `GraduatedBlocker`.

## Suggested Fix

Share the `catching_up` `Arc<AtomicBool>` between the `GraduatedBlocker` and the `RevmApplication`. When `RevmApplication` detects that catch-up is complete, clear the shared flag:

**In `runner.rs` -- pass the same `Arc<AtomicBool>` to the application:**

```rust
let resolver_catching_up = Arc::new(AtomicBool::new(recovered_head_height.is_some()));
let resolver_blocker = GraduatedBlocker::new(transport.oracle.clone(), resolver_catching_up.clone());
// Pass resolver_catching_up to the application
app = app.with_resolver_catching_up(resolver_catching_up);
```

**In `app.rs` -- clear the flag when catch-up completes (around line 699-706):**

```rust
if prev_verified < self.recovered_height.load(Ordering::Relaxed)
    && block.height >= self.recovered_height.load(Ordering::Relaxed)
{
    info!(
        height = block.height,
        recovered_height = self.recovered_height.load(Ordering::Relaxed),
        "catch-up complete, enabling peer banning"
    );
    // Clear the GraduatedBlocker's flag
    if let Some(ref flag) = self.resolver_catching_up {
        flag.store(false, Ordering::Release);
    }
}
```

## Files to Modify

- `crates/node/runner/src/runner.rs:1348-1350` -- Pass the `Arc<AtomicBool>` to `RevmApplication`
- `crates/node/runner/src/app.rs:90-121` -- Add `resolver_catching_up: Option<Arc<AtomicBool>>` field
- `crates/node/runner/src/app.rs:699-706` -- Clear the flag when catch-up is complete

## Related Issues

- [008 -- Catch-Up Silent State Divergence](./008-catch-up-silent-state-divergence.md) -- Related catch-up recovery issue; both involve the catch-up state machine
- [005 -- Memory Exhaustion](./005-memory-exhaustion-devnet.md) -- OOM kills trigger restarts, which activate this bug
