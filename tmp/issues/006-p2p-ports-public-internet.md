# 006: P2P Ports Exposed on 0.0.0.0 to Public Internet

**Category:** security / deployment
**Severity:** critical
**Labels:** security, docker, p2p

---

## Summary

The P2P ports (30400-30409) on the live 10-node devnet are bound to `0.0.0.0`, making them directly accessible from the public internet. For a single-host devnet where all P2P traffic is intra-container via the Docker bridge network, exposing these ports externally is unnecessary and creates attack surface. In contrast, the RPC (8545-8554) and metrics (9000-9009) ports are correctly bound to `127.0.0.1`.

## Problem

The Docker compose file at `docker/compose/devnet.yaml` maps P2P ports using the format `"30400:30303"` without specifying a bind address prefix. Docker defaults this to `0.0.0.0`, meaning the port is accessible from any network interface, including the public internet.

Verified on the live devnet via `ss -tlnp`:
```
LISTEN 0  4096  0.0.0.0:30400  0.0.0.0:*   # node0
LISTEN 0  4096  0.0.0.0:30401  0.0.0.0:*   # node1
...
LISTEN 0  4096  0.0.0.0:30409  0.0.0.0:*   # node9
```

The same pattern exists in the 4-node compose file in the repository at `docker/compose/devnet.yaml`. All four nodes expose P2P ports without a bind address:

- Line 247: `"30400:30303"` (node0)
- Line 275: `"30401:30303"` (node1)
- Line 304: `"30402:30303"` (node2)
- Line 333: `"30403:30303"` (node3)

## Code Reference

**4-node compose P2P port mappings -- `docker/compose/devnet.yaml`:**

```yaml
# node0 (line 247)
ports:
  - "30400:30303"  # P2P -- no bind address, defaults to 0.0.0.0

# node1 (line 275)
ports:
  - "30401:30303"  # P2P

# node2 (line 304)
ports:
  - "30402:30303"  # P2P

# node3 (line 333)
ports:
  - "30403:30303"  # P2P
```

For comparison, RPC and metrics ports in the same file correctly use loopback binding:
```yaml
  - "127.0.0.1:8545:8545"  # RPC
  - "127.0.0.1:9000:9000"  # Metrics
```

## Impact

An external attacker can:

1. **Flood P2P connections**: Exhaust connection limits and file descriptors, preventing legitimate validator-to-validator communication.
2. **Send malformed protocol messages**: Trigger parsing bugs or panics in the commonware P2P stack.
3. **Consume bandwidth**: The P2P rate quota is 1000 messages/second per channel (configured in `crates/node/runner/src/runner.rs`), allowing significant bandwidth consumption from unauthenticated sources.
4. **Attempt protocol-level attacks**: While the P2P layer uses authenticated channels (commonware discovery with Ed25519 keys), the attacker can still waste resources on handshake failures and connection churn.

The risk is amplified on the Hetzner bare-metal server where the devnet runs, as the server's public IP is directly reachable from the internet without any cloud-provider firewall.

## Root Cause

The Docker compose port mapping uses the format `"30400:30303"` without a `127.0.0.1:` prefix. Docker defaults unqualified port mappings to `0.0.0.0`. The developers correctly bound RPC and metrics ports to loopback but missed the P2P ports.

## Suggested Fix

Change the port mappings to bind to loopback for single-host deployments:

**Before:**
```yaml
ports:
  - "30400:30303"  # P2P
```

**After:**
```yaml
ports:
  - "127.0.0.1:30400:30303"  # P2P
```

For multi-host deployments where P2P ports must be externally reachable, use a firewall (iptables/nftables) to restrict P2P access to known validator IP addresses rather than relying on Docker port bindings.

## Files to Modify

- `docker/compose/devnet.yaml` -- Lines 247, 275, 304, 333: Add `127.0.0.1:` prefix to P2P port mappings
- `docker/compose/devnet-10node.yaml` (on server at `/opt/kora/docker/compose/`) -- Same fix for all 10 nodes

## Related Issues

- [002 -- DKG Plaintext TCP Shares](./002-dkg-plaintext-tcp-shares.md) -- Related network security concern; DKG also uses unprotected network channels
