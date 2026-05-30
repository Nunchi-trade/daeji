# Firewall `trusted_ips` Defaults to `0.0.0.0/0` -- RPC, Metrics, and Grafana Open to Internet

**Category:** Security -- Deployment/Infrastructure
**Severity:** High

## Summary

The Ansible inventory defaults the `trusted_ips` firewall variable to `0.0.0.0/0`, which allows all internet traffic to reach management ports including JSON-RPC (8545-8548), Prometheus metrics (9000-9003, 9090), and Grafana (3000). Any server provisioned without explicitly overriding this variable exposes its full RPC and monitoring stack to the public internet. Combined with the default Grafana password of `admin` (issue 182), this gives unauthenticated admin access to the monitoring stack.

## Problem

Kora uses Ansible to provision remote devnet servers. The firewall is configured via an nftables template (`ansible/roles/firewall/templates/nftables.conf.j2`) that generates per-IP allow rules from a `trusted_ips` variable. This variable is defined in the Ansible group vars at `ansible/inventory/group_vars/devnet.yml`, lines 36-39, and its default value is `0.0.0.0/0` -- a CIDR range that matches every IPv4 address on the internet.

The nftables template iterates over `trusted_ips` to create source-IP-restricted rules for four categories of ports:

| Port(s)     | Service         | Template Lines | Risk                                                    |
|-------------|-----------------|----------------|---------------------------------------------------------|
| 8545-8548   | JSON-RPC        | 31-39          | Submit transactions, query state, DoS via `eth_getLogs` |
| 9000-9003   | Validator Metrics | 41-49        | Internal operational data aids targeted attacks         |
| 9090        | Prometheus      | 51-59          | Full metric history, topology, alert rules              |
| 3000        | Grafana         | 61-69          | Dashboard admin with default `admin/admin` credentials  |

When `trusted_ips` contains `0.0.0.0/0`, every generated `ip saddr` rule matches all traffic, making the IP restriction functionally nonexistent. The template has an `else` branch for when `trusted_ips` is empty that also opens the ports (with a WARNING comment), so an empty list is also not safe -- but `0.0.0.0/0` is worse because it gives the appearance of having a firewall rule while providing no restriction.

Note: P2P ports (30400-30403, 30500) are intentionally open to the world at lines 23-29 of the template and are not affected by `trusted_ips`. This issue is distinct from issue 006 which covers P2P port exposure.

**File:** `ansible/inventory/group_vars/devnet.yml`, lines 36-39
**File:** `ansible/roles/firewall/templates/nftables.conf.j2`, lines 31-69

## Code Reference

The default variable definition:

```yaml
# ansible/inventory/group_vars/devnet.yml:36-39
# Trusted IPs allowed to access RPC, metrics, Prometheus, and Grafana.
# CHANGEME: restrict to your operator/monitoring IPs for production.
trusted_ips:
  - "0.0.0.0/0"  # WARNING: allows all traffic. Replace with specific IPs.
```

The nftables template that consumes this variable (showing the RPC section as an example; Metrics, Prometheus, and Grafana sections follow the same pattern):

```jinja2
# ansible/roles/firewall/templates/nftables.conf.j2:31-39
        # Kora RPC (restricted to trusted IPs)
{% if trusted_ips | default([]) | length > 0 %}
{% for ip in trusted_ips %}
        ip saddr {{ ip }} tcp dport { {{ rpc_ports | replace(':', '-') }} } accept
{% endfor %}
{% else %}
        # WARNING: RPC is open to the world. Set 'trusted_ips' to restrict access.
        tcp dport { {{ rpc_ports | replace(':', '-') }} } accept
{% endif %}
```

With the default value, this renders as:

```
        ip saddr 0.0.0.0/0 tcp dport { 8545-8548 } accept
```

This rule matches all IPv4 source addresses.

## Impact

1. **Full RPC access from the internet.** Anyone can submit transactions, query blockchain state, and exhaust node resources via expensive calls like `eth_getLogs` (see issue 023, which notes `eth_getLogs` has no result limit) or `eth_call` (see issue 013, which notes `eth_call` blocks the async runtime).
2. **Prometheus metrics exposed.** Internal operational data (memory usage, block heights, peer topology, nullification rates, consensus view numbers) is visible to attackers and provides intelligence for targeted attacks (e.g., knowing exactly when a node is struggling or which validators are reachable).
3. **Grafana admin access with default credentials.** When combined with issue 182 (Grafana password hardcoded as `admin`), an attacker gets full admin access to the monitoring stack. A Grafana admin can modify dashboards, suppress alert rules, add data sources, and execute arbitrary PromQL queries.
4. **Silent exposure.** The `CHANGEME` comment in the YAML file is easy to overlook during rapid deployment. The generated nftables rules contain `ip saddr 0.0.0.0/0` which looks like a legitimate firewall rule but provides no protection.

## Root Cause

The `group_vars/devnet.yml` sets `trusted_ips` to a permissive default (`0.0.0.0/0`) for developer convenience during initial devnet setup. The nftables template trusts this variable to define allowed source IPs for all management ports. There is no validation step in any Ansible playbook that rejects `0.0.0.0/0` before applying the firewall rules.

## Suggested Fix

1. **Change the default to require explicit configuration.** Set `trusted_ips` to an empty list and modify the nftables template's `else` branch to DROP rather than ACCEPT when no trusted IPs are defined:

   ```yaml
   # BEFORE (ansible/inventory/group_vars/devnet.yml:38-39)
   trusted_ips:
     - "0.0.0.0/0"  # WARNING: allows all traffic. Replace with specific IPs.

   # AFTER
   trusted_ips: []
   # Override in host_vars or via -e to allow access:
   #   trusted_ips: ["10.0.0.0/8", "203.0.113.50/32"]
   ```

2. **Fix the template's `else` branch** to not fall through to open access when the list is empty:

   ```jinja2
   # BEFORE (nftables.conf.j2:36-38)
   {% else %}
           # WARNING: RPC is open to the world. Set 'trusted_ips' to restrict access.
           tcp dport { {{ rpc_ports | replace(':', '-') }} } accept
   {% endif %}

   # AFTER
   {% else %}
           # No trusted IPs configured -- RPC blocked. Set 'trusted_ips' to allow access.
   {% endif %}
   ```

3. **Add a validation pre-task** in the deploy or provision playbook:

   ```yaml
   - name: Validate trusted_ips is not wide open
     ansible.builtin.fail:
       msg: >
         trusted_ips contains 0.0.0.0/0, which allows all internet traffic to
         RPC/metrics/Grafana. Override in host_vars with specific IPs.
     when: "'0.0.0.0/0' in (trusted_ips | default([]))"
   ```

## Files to Modify

- `ansible/inventory/group_vars/devnet.yml` -- change `trusted_ips` default from `["0.0.0.0/0"]` to `[]`
- `ansible/roles/firewall/templates/nftables.conf.j2` -- change `else` branches (lines 36-38, 46-48, 56-58, 66-68) to drop rather than accept when no trusted IPs are configured
- `ansible/playbooks/provision.yml` or `ansible/playbooks/deploy.yml` -- add a pre-task validation for `trusted_ips`

## Related Issues

- [182 - Grafana admin password hardcoded as "admin"](/Users/will/dev/nunchi/daeji/tmp/kora/issues/182-grafana-admin-password-hardcoded.md) -- combined with this issue, gives unauthenticated admin access to Grafana from the internet
- [006 - P2P ports exposed on 0.0.0.0](/Users/will/dev/nunchi/daeji/tmp/kora/issues/006-p2p-ports-public-internet.md) -- related P2P layer exposure (separate from management ports covered here)
- [023 - eth_getLogs no result limit DoS](/Users/will/dev/nunchi/daeji/tmp/kora/issues/023-getlogs-no-result-limit-dos.md) -- amplifies the RPC exposure: no result limit means a single query can exhaust node memory
- [013 - eth_call blocks async runtime](/Users/will/dev/nunchi/daeji/tmp/kora/issues/013-eth-call-blocks-async-runtime.md) -- amplifies the RPC exposure: a single call can block the async runtime

## Labels

`security`, `config`, `docker`, `reliability`
