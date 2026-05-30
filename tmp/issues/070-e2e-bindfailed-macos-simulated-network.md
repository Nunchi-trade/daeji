# E2E Tests Fail on macOS: BindFailed for Random 127.x.x.x Loopback Addresses

**Category**: bug -- testing / platform compatibility
**Severity**: high

**Labels**: `bug`, `ci`, `p2p`, `docker`

---

## Summary

All 33 end-to-end tests fail immediately on macOS (aarch64/Apple Silicon) because the simulated P2P network generates random `127.x.x.x` loopback addresses for socket binding. On Linux, the entire `127.0.0.0/8` subnet is routable by default, but on macOS only `127.0.0.1` is configured on the loopback interface. Binding to any other `127.x.x.x` address (e.g., `127.42.13.7`) fails with a `BindFailed` error, causing a panic in the simulated network.

---

## Problem

Kora's E2E test harness uses a `SimContext` wrapper around the commonware runtime that generates a random loopback base address for simulated networking. The `SimContext::new()` method computes a random `127.x.x.x` address:

```rust
let seed = rng.next_u32() ^ std::process::id();
let base_addr = Ipv4Addr::new(127, (seed >> 16) as u8, (seed >> 8) as u8, seed as u8);
```

This address is used when the simulated network binds sockets. The `bind()` method in `SimContext`'s `Network` implementation remaps ports but preserves the base address:

```rust
fn bind(&self, socket: SocketAddr) -> impl Future<Output = Result<Self::Listener, Error>> + Send {
    self.inner.bind(remap_socket(socket, self.port_offset))
}
```

On Linux, the kernel automatically routes all addresses in the `127.0.0.0/8` range to the loopback interface. On macOS, only `127.0.0.1` is configured by default -- binding to `127.42.13.7` fails because there is no network interface for that address.

The `force_base_addr` flag in `SimContext` is used to inject the base address via the `RngCore::next_u32()` override (line 218-221), which the commonware `simulated::Network::new()` calls to determine its base address. The first `next_u32()` call returns the pre-computed `base_addr` bits, after which the flag is cleared.

---

## Code Reference

**SimContext generating random loopback** -- `/Users/will/dev/nunchi/daeji/crates/network/transport-sim/src/context.rs:49-58`:
```rust
impl SimContext {
    /// Create a new simulation context wrapping a tokio context.
    pub fn new(inner: tokio::Context) -> Self {
        let mut rng = OsRng;
        let span = u32::from(PORT_BASE_MAX - PORT_BASE_MIN + 1);
        let base = PORT_BASE_MIN + (rng.next_u32() % span) as u16;
        let seed = rng.next_u32() ^ std::process::id();
        let base_addr = Ipv4Addr::new(127, (seed >> 16) as u8, (seed >> 8) as u8, seed as u8);
        Self { inner, force_base_addr: true, base_addr, port_offset: base }
    }
}
```

**RngCore override that injects the base address** -- `/Users/will/dev/nunchi/daeji/crates/network/transport-sim/src/context.rs:216-224`:
```rust
impl RngCore for SimContext {
    fn next_u32(&mut self) -> u32 {
        if self.force_base_addr {
            self.force_base_addr = false;
            return self.base_addr.to_bits();
        }
        let mut rng = OsRng;
        RngCore::next_u32(&mut rng)
    }
```

**E2E harness creates the SimContext** -- `/Users/will/dev/nunchi/daeji/crates/e2e/src/harness.rs:222-227`:
```rust
        let (network, oracle) = simulated::Network::new(
            SimContext::new(context.child("network")),
            simulated::Config {
                max_size: MAX_MSG_SIZE as u32,
                disconnect_on_block: true,
                tracked_peer_sets: NZUsize!(4),
```

---

## Error Output

When running E2E tests on macOS:
```
thread 'tokio-rt-worker' panicked at commonware-p2p-.../src/simulated/network.rs:...:
called `Result::unwrap()` on an `Err` value: BindFailed
```

The first test (`test_balance_updates_after_finalization`) fails, and the remaining 32 tests are cancelled.

---

## Impact

- **All 33 E2E tests are blocked on macOS** (the primary development platform for many contributors)
- The first test failure causes the entire test suite to be cancelled
- CI on Ubuntu/Linux is likely unaffected because Linux routes the entire `127.0.0.0/8` subnet
- Docker-based devnet deployment is unaffected (runs Linux in container)
- Developers on macOS cannot run E2E tests locally, reducing confidence in changes before pushing to CI

---

## Root Cause

The `SimContext` generates a random loopback base address to allow parallel test execution without port conflicts (different tests get different `127.x.x.x` addresses). This relies on the Linux kernel's behavior of automatically routing all `127.0.0.0/8` addresses to the loopback interface. macOS does not configure these addresses by default -- only `127.0.0.1` is available on the `lo0` interface.

---

## Suggested Fix

**Option A (recommended -- fix in SimContext)**: Detect macOS at compile time and force `127.0.0.1` as the base address, relying solely on port offsets for isolation:

```rust
impl SimContext {
    pub fn new(inner: tokio::Context) -> Self {
        let mut rng = OsRng;
        let span = u32::from(PORT_BASE_MAX - PORT_BASE_MIN + 1);
        let base = PORT_BASE_MIN + (rng.next_u32() % span) as u16;

        #[cfg(target_os = "macos")]
        let base_addr = Ipv4Addr::LOCALHOST;

        #[cfg(not(target_os = "macos"))]
        let base_addr = {
            let seed = rng.next_u32() ^ std::process::id();
            Ipv4Addr::new(127, (seed >> 16) as u8, (seed >> 8) as u8, seed as u8)
        };

        Self { inner, force_base_addr: true, base_addr, port_offset: base }
    }
}
```

The port offset randomization (range 40000-64511) already provides sufficient isolation for parallel test execution on macOS. On Linux, the address randomization provides an additional layer of isolation.

**Option B (workaround -- configure macOS loopback)**: Before running tests, add loopback aliases:
```bash
for i in $(seq 0 255); do
    for j in $(seq 0 255); do
        sudo ifconfig lo0 alias 127.0.$i.$j
    done
done
```
This is fragile and requires root access, making it unsuitable as a permanent solution.

**Option C (alternative)**: Always use `127.0.0.1` regardless of platform, and increase the port offset range to provide sufficient isolation:
```rust
let base_addr = Ipv4Addr::LOCALHOST; // Always 127.0.0.1
```
This is the simplest fix but slightly reduces isolation between parallel test processes compared to address-based isolation on Linux.

---

## Files to Modify

- `/Users/will/dev/nunchi/daeji/crates/network/transport-sim/src/context.rs` -- fix `SimContext::new()` to use `127.0.0.1` on macOS

---

## Related Issues

- `046-p2p-production-runner-uses-local-transport.md` -- related to transport configuration
