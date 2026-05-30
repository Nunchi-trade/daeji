# nftables Forward Chain Blocks Docker Compose Bridge Traffic

**Category:** Bug -- Deployment/Infrastructure
**Severity:** Medium

## Summary

The nftables firewall template's forward chain only allows traffic on the `docker0` bridge interface, but Docker Compose creates custom bridge networks with different interface names (e.g., `br-xxxx` for the `kora-net` network). On systems where nftables is the primary packet filter, inter-container traffic on Docker Compose networks is silently dropped by the firewall, breaking validator-to-validator P2P communication, Prometheus metric scraping, and Loki log aggregation. This currently works "by accident" on some systems where Docker's own iptables rules take precedence, but breaks on fresh installations with nftables as the sole firewall.

## Problem

Kora's Ansible provisioning role deploys an nftables firewall configuration via a Jinja2 template. The forward chain (which controls traffic routed between network interfaces, including Docker bridge interfaces) has a default policy of `drop` and only allows traffic between interfaces named `docker0`:

```
chain forward {
    type filter hook forward priority 0; policy drop;
    iifname "docker0" oifname "docker0" accept
    ct state established,related accept
}
```

However, Docker Compose creates a dedicated bridge network for inter-container communication. The Kora compose file (`docker/compose/devnet.yaml`, line 4) defines a `kora-net` bridge network:

```yaml
networks:
  kora-net:
    driver: bridge
```

Docker assigns this network an interface name like `br-<12-char-hex>` (e.g., `br-a1b2c3d4e5f6`), not `docker0`. The `docker0` bridge is only used for containers that use the default bridge network, which Docker Compose containers do not.

When the nftables ruleset is applied (the template starts with `flush ruleset` at line 3, which removes all existing rules including Docker's auto-generated iptables rules), any traffic between containers on `kora-net` that traverses the forward chain is dropped because neither the input nor output interface matches `docker0`.

This affects:
1. **Validator-to-validator P2P traffic.** Nodes communicate via hostnames (`node0`, `node1`, etc.) resolved by Docker's embedded DNS to container IPs on the `kora-net` bridge.
2. **Prometheus to validator metric scraping.** Prometheus scrapes `validator-node0:9002` through `validator-node3:9002` (see `docker/config/prometheus.yml`, lines 16-20).
3. **Promtail to Loki log shipping.** Promtail sends logs to `loki:3100` over the same bridge.

**File:** `ansible/roles/firewall/templates/nftables.conf.j2`, lines 75-81

## Code Reference

```
# ansible/roles/firewall/templates/nftables.conf.j2:1-3
#!/usr/sbin/nft -f

flush ruleset                              # <-- Removes Docker's auto-generated iptables rules

# ... (lines 5-73: input chain rules) ...

# ansible/roles/firewall/templates/nftables.conf.j2:75-81
    chain forward {
        type filter hook forward priority 0; policy drop;

        # Allow Docker bridge traffic
        iifname "docker0" oifname "docker0" accept   # <-- Only matches default bridge, not Compose bridges
        ct state established,related accept
    }
```

The Docker Compose network definition:

```yaml
# docker/compose/devnet.yaml:3-5
networks:
  kora-net:
    driver: bridge
```

## Impact

1. **Complete inter-container communication failure on fresh nftables-based systems.** On a server where nftables is the primary firewall (the intended configuration, given the `flush ruleset` at line 3 of the template), all container-to-container traffic on the Compose bridge network is dropped. Validators cannot communicate, Prometheus cannot scrape metrics, and Promtail cannot ship logs.
2. **Intermittent failures depending on Docker version.** Some Docker versions/configurations manage their own iptables rules in a separate table that may not be affected by `flush ruleset`. This means the firewall may "work" on one server but break on another with a different Docker version or nftables/iptables backend configuration.
3. **Difficult to diagnose.** The failure manifests as network timeouts between containers (e.g., P2P connection failures, Prometheus scrape timeouts) rather than an obvious firewall error. The nftables logs (`[nftables drop]` prefix, line 72) would show the drops, but only if the operator knows to check `journalctl` for nftables messages rather than container logs.

## Root Cause

The nftables forward chain was written with the assumption that Docker uses the `docker0` bridge for all inter-container traffic. This is only true for containers using Docker's default bridge network. Docker Compose creates custom bridge networks with auto-generated interface names (`br-*`) that are not covered by the `iifname "docker0"` rule.

## Suggested Fix

**Option A: Add a wildcard rule for Docker Compose bridges:**

```diff
# ansible/roles/firewall/templates/nftables.conf.j2:75-81
    chain forward {
        type filter hook forward priority 0; policy drop;

        # Allow Docker bridge traffic (default bridge and Compose custom bridges)
        iifname "docker0" oifname "docker0" accept
+       iifname "br-*" oifname "br-*" accept
        ct state established,related accept
    }
```

**Option B: Allow forwarding on all Docker-managed interfaces** by using the `docker` chain approach:

```
    chain forward {
        type filter hook forward priority 0; policy drop;

        # Allow all traffic on Docker-managed bridges
        iifname "docker*" accept
        iifname "br-*" accept
        oifname "docker*" accept
        oifname "br-*" accept
        ct state established,related accept
    }
```

**Option C: Use Docker's `--iptables=true` (default) and avoid `flush ruleset`:**

Instead of `flush ruleset`, only flush the `inet filter` table to avoid destroying Docker's auto-managed rules:

```
# Instead of: flush ruleset
delete table inet filter
```

Option A is the simplest fix with the least side effects.

## Files to Modify

- `ansible/roles/firewall/templates/nftables.conf.j2` -- add `br-*` interface matching to the forward chain (lines 75-81)

## Related Issues

- [181 - Firewall trusted_ips defaults to 0.0.0.0/0](/Users/will/dev/nunchi/daeji/tmp/kora/issues/181-firewall-trusted-ips-open-to-internet.md) -- another firewall misconfiguration in the same nftables template
- [006 - P2P ports exposed on 0.0.0.0](/Users/will/dev/nunchi/daeji/tmp/kora/issues/006-p2p-ports-public-internet.md) -- P2P port exposure at the input chain level (separate from this forward chain issue)

## Labels

`bug`, `docker`, `p2p`, `config`, `reliability`
