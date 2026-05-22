# No firewall rules -- all ports (P2P, RPC, metrics) publicly exposed on devnet

## Summary

The devnet Docker Compose configuration publishes all service ports to `0.0.0.0` by default, meaning the JSON-RPC endpoints, Prometheus metrics, Grafana dashboards, and internal P2P ports are all reachable from any IP on the internet. On the production devnet host (65.21.232.29), there are no iptables/nftables firewall rules restricting access. This allows anyone to submit transactions, query state, scrape metrics, and potentially interfere with consensus by sending malformed P2P messages.

## Priority

**P1 -- Operational Reliability**

While this is a devnet (not mainnet), the publicly exposed RPC port allows arbitrary transaction submission and state queries, the metrics port leaks operational details, and the Grafana dashboard (with default admin/admin credentials) provides a monitoring attack surface. For a devnet running on a public Hetzner host, these ports should be restricted.

## Problem Description

### Exposed ports in Docker Compose

**File**: `docker/compose/devnet.yaml`

Every validator publishes three ports to `0.0.0.0` (the Docker default):

```yaml
# validator-node0 (lines 217-219)
    ports:
      - "30400:30303"   # P2P
      - "8545:8545"     # JSON-RPC
      - "9000:9002"     # Prometheus metrics

# validator-node1 (lines 239-241)
    ports:
      - "30401:30303"
      - "8546:8545"
      - "9001:9002"

# validator-node2 (lines 263-265)
    ports:
      - "30402:30303"
      - "8547:8545"
      - "9002:9002"

# validator-node3 (lines 285-287)
    ports:
      - "30403:30303"
      - "8548:8545"
      - "9003:9002"
```

The secondary node publishes P2P (line 308):

```yaml
# secondary-node0 (line 308)
    ports:
      - "30500:30303"
```

The observability stack publishes (lines 323, 335, 367):

```yaml
# prometheus (line 323)
    ports:
      - "9090:9090"     # Prometheus web UI + API

# loki (line 335)
    ports:
      - "3100:3100"     # Loki log API

# grafana (line 367)
    ports:
      - "3000:3000"     # Grafana web UI (admin/admin by default)
```

### Total exposed attack surface

| Port Range | Service | Protocol | Risk |
|------------|---------|----------|------|
| 8545-8548 | JSON-RPC (4 validators) | HTTP | Arbitrary tx submission, state queries, `eth_sendRawTransaction` |
| 9000-9003 | Prometheus metrics (4 validators) | HTTP | Operational information leak (peer count, block height, error rates) |
| 30400-30403 | P2P (4 validators) | TCP | Consensus message injection, connection exhaustion |
| 30500 | P2P (secondary) | TCP | Same as above |
| 9090 | Prometheus | HTTP | Full metrics API, PromQL queries, target discovery |
| 3100 | Loki | HTTP | Log API (all node logs queryable) |
| 3000 | Grafana | HTTP | Dashboard UI, default admin/admin credentials |

That is **14 ports** exposed to the public internet on a single host.

### Grafana default credentials

**File**: `docker/compose/devnet.yaml`, lines 362-365

```yaml
    environment:
      - GF_SECURITY_ADMIN_USER=admin
      - GF_SECURITY_ADMIN_PASSWORD=${GF_SECURITY_ADMIN_PASSWORD:-admin}
      - GF_AUTH_ANONYMOUS_ENABLED=true
      - GF_AUTH_ANONYMOUS_ORG_ROLE=Viewer
```

The Grafana admin password defaults to `admin` unless overridden by the environment variable. Anonymous access is enabled as Viewer, meaning anyone who hits port 3000 can view all dashboards without authentication.

### Docker's default port binding

When a port is specified as `"8545:8545"` (without a bind address), Docker publishes it to `0.0.0.0:8545`, which means it listens on all network interfaces. This bypasses any host-level iptables rules because Docker manipulates iptables directly with its own chains.

## Fix

### Step 1: Bind non-P2P ports to localhost

For services that only need to be accessed locally (or via SSH tunnel), bind to `127.0.0.1`:

```yaml
# validator-node0
    ports:
      - "30400:30303"           # P2P: must remain public for inter-node communication
      - "127.0.0.1:8545:8545"   # RPC: localhost only
      - "127.0.0.1:9000:9002"   # Metrics: localhost only (Prometheus scrapes via Docker network)
```

Repeat for all four validators, adjusting port numbers accordingly.

### Step 2: Move Prometheus scraping to Docker network

Prometheus already runs on the `kora-net` Docker network alongside the validators. It can scrape metrics via the internal Docker DNS names (e.g., `node0:9002`) instead of published ports. Remove the metrics port publishing from validators entirely:

```yaml
# validator-node0
    ports:
      - "30400:30303"           # P2P only
      - "127.0.0.1:8545:8545"   # RPC: localhost only
      # Metrics port NOT published -- Prometheus scrapes via kora-net
```

Update `docker/config/prometheus.yml` to use internal hostnames if not already configured.

### Step 3: Bind observability ports to localhost

```yaml
# prometheus
    ports:
      - "127.0.0.1:9090:9090"

# loki
    ports:
      - "127.0.0.1:3100:3100"

# grafana
    ports:
      - "127.0.0.1:3000:3000"
```

Access Grafana and Prometheus via SSH tunnel: `ssh -L 3000:localhost:3000 user@65.21.232.29`

### Step 4: Set a real Grafana admin password

In the deployment's `.env` file or Ansible variables:

```
GF_SECURITY_ADMIN_PASSWORD=<strong-random-password>
```

### Step 5: Consider host firewall rules (defense in depth)

Even with Docker port bindings restricted, add UFW or nftables rules as defense in depth:

```bash
# Allow SSH
ufw allow 22/tcp
# Allow P2P ports for validators
ufw allow 30400:30403/tcp
ufw allow 30500/tcp
# Deny everything else inbound
ufw default deny incoming
ufw enable
```

Note: Docker bypasses UFW by default because it manipulates iptables directly. To make UFW effective with Docker, configure Docker to not manipulate iptables (set `"iptables": false` in `/etc/docker/daemon.json`) or use `DOCKER_IPTABLES=false`. The `127.0.0.1` bind approach in Steps 1-3 is more reliable.

## Effort Estimate

**1-2 hours**:
- 15 minutes to update port bindings in `devnet.yaml`
- 15 minutes to verify Prometheus can still scrape via internal network
- 15 minutes to set Grafana password and test SSH tunnel access
- 15-30 minutes to update Ansible deployment if needed
- 30 minutes for validation testing

## Affected Files

| File | Change |
|------|--------|
| `docker/compose/devnet.yaml` | Bind RPC/metrics/observability ports to `127.0.0.1` |
| `docker/config/prometheus.yml` | Verify scrape targets use Docker-internal hostnames |
| `.env` or Ansible vars | Set `GF_SECURITY_ADMIN_PASSWORD` |
| `ansible/playbooks/deploy.yml` | Update if port changes affect deployment |

## Testing Checklist

- [ ] Bind RPC ports (8545-8548) to `127.0.0.1` in `devnet.yaml`
- [ ] Bind metrics ports (9000-9003) to `127.0.0.1` or remove entirely
- [ ] Bind Prometheus (9090), Loki (3100), Grafana (3000) to `127.0.0.1`
- [ ] Verify Prometheus can still scrape validator metrics via Docker network
- [ ] Verify Grafana dashboards load correctly via SSH tunnel
- [ ] Verify RPC is accessible via SSH tunnel: `curl http://localhost:8545`
- [ ] Verify RPC is NOT accessible from external IP: `curl http://65.21.232.29:8545` should fail
- [ ] Set non-default Grafana admin password
- [ ] Run loadgen via SSH tunnel to confirm RPC functionality
