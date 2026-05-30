# RPC, P2P Networking, and Docker Infrastructure

## 1. RPC Layer

**Directory:** `crates/node/rpc/`

### Endpoints

#### Ethereum API (`eth_*`)
| Endpoint | File:Line | Purpose |
|----------|-----------|---------|
| `eth_chainId` | eth.rs | Returns chain ID |
| `eth_blockNumber` | eth.rs | Current block number |
| `eth_getBalance` | eth.rs | Account balance |
| `eth_getTransactionCount` | eth.rs | Account nonce |
| `eth_getCode` | eth.rs | Contract bytecode |
| `eth_getStorageAt` | eth.rs | Storage slot value |
| `eth_sendRawTransaction` | eth.rs:299-309 | **Submit transaction to mempool** |
| `eth_call` | eth.rs | Execute call without tx |
| `eth_estimateGas` | eth.rs | Gas estimation |
| `eth_getBlockByNumber` | eth.rs | Block by number |
| `eth_getBlockByHash` | eth.rs | Block by hash |
| `eth_getTransactionByHash` | eth.rs | Transaction details |
| `eth_getTransactionReceipt` | eth.rs | Transaction receipt |
| `eth_gasPrice` | eth.rs | Returns 1 Gwei (hardcoded) |
| `eth_maxPriorityFeePerGas` | eth.rs | Returns 1 Gwei (hardcoded) |
| `eth_feeHistory` | eth.rs | Fee history |
| `eth_accounts` | eth.rs | Empty (non-wallet) |
| `eth_protocolVersion` | eth.rs | Returns "0x44" |
| `eth_syncing` | eth.rs | Always false |
| `eth_getLogs` | eth.rs | Log query |

#### Net API (`net_*`)
| Endpoint | Purpose |
|----------|---------|
| `net_version` | Chain ID as string |
| `net_listening` | Always true |
| `net_peerCount` | Connected peers |

#### Web3 API (`web3_*`)
| Endpoint | Purpose |
|----------|---------|
| `web3_clientVersion` | "kora/0.1.0" |
| `web3_sha3` | Keccak-256 hash |

#### Kora API (`kora_*`)
| Endpoint | Purpose |
|----------|---------|
| `kora_nodeStatus` | Consensus info (view, finalized, leader) |

#### HTTP Endpoints
| Endpoint | Purpose |
|----------|---------|
| `GET /health` | Returns "ok" |
| `GET /status` | JSON node status |

### Transaction Submission Path (eth.rs:299-309)

```rust
async fn send_raw_transaction(&self, data: Bytes) -> RpcResult<B256> {
    let tx_hash = keccak256(&data);
    let pending_tx = raw_tx_to_pending_rpc(&data)?;      // Decode & validate

    if let Some(ref submit) = self.tx_submit {
        submit(data).await?;                               // Submit to mempool
    }

    self.pending_txs.write().await.insert(tx_hash, pending_tx);  // Cache for queries
    Ok(tx_hash)
}
```

**ISSUE:** If `tx_submit` callback is `None`, transaction is accepted and cached for queries but never reaches the mempool. This is a silent drop.

### Rate Limiting — CONFIGURED BUT NOT ENFORCED

**Configuration** (rpc/config.rs:130-150):
```rust
pub struct RateLimitConfig {
    pub requests_per_second: u64,  // Default: 100
    pub burst_size: u64,           // Default: 200
}
```

**Status:** The `RateLimitConfig` is defined and stored in `RpcServerConfig`, but it's **never applied** to the HTTP or jsonrpsee server. Only `ConcurrencyLimitLayer` is used (server.rs:219):

```rust
.layer(ConcurrencyLimitLayer::new(max_connections as usize))
```

### Request Body Size — NO EXPLICIT LIMIT

No `max_payload_size` is set on the jsonrpsee `ServerBuilder`. The default limit comes from the library (likely 10 MB or configurable).

### Connection Limits

- Default: 100 concurrent connections
- Applied via both `ConcurrencyLimitLayer` and `ServerBuilder.max_connections()`

---

## 2. P2P Networking

**Directory:** `crates/network/`

### Channel Definitions (transport/channels.rs:9-22)

| Channel | ID | Purpose |
|---------|----|---------|
| `CHANNEL_VOTES` | 0 | Consensus votes (leader → validators) |
| `CHANNEL_CERTS` | 1 | Finalization certificates |
| `CHANNEL_RESOLVER` | 2 | Dependency fetch control |
| `CHANNEL_BLOCKS` | 3 | Full block broadcast |
| `CHANNEL_BACKFILL` | 4 | Block backfill responses |

### Channel Rate Limiting (transport/builder.rs:21-87)

```rust
const fn default_quota() -> Quota {
    Quota::per_second(NonZeroU32::new(1000).expect("1000 is non-zero"))
}
```

All 5 channels use: **1000 messages/second** per channel, **256-item backlog**.

### Channel Groups

```rust
pub struct SimplexChannels {
    pub votes: (Sender, Receiver),      // Consensus votes
    pub certs: (Sender, Receiver),      // Certificates
    pub resolver: (Sender, Receiver),   // Resolver control
}

pub struct MarshalChannels {
    pub blocks: (Sender, Receiver),     // Block broadcast
    pub backfill: (Sender, Receiver),   // Block backfill
}
```

### Resolver Configuration (marshal/peers.rs:32-50)

| Parameter | Value |
|-----------|-------|
| Mailbox size | 1024 |
| Initial delay | 200ms |
| Timeout | 200ms |
| Fetch retry timeout | 100ms |

### Known P2P Issues

1. **Resolver blocks peers on "invalid data"**: When a restarted node requests historical data, the resolver can reject it as "invalid" and block that peer. This prevents catch-up.

2. **Certificate rejection during catch-up**: A node that's behind may receive certificates for future views. The resolver may reject valid lower-view certificates, stalling catch-up.

3. **Peer authentication**: Uses ED25519 with namespace `b"_COMMONWARE_KORA_NETWORK"` via Commonware's authenticated discovery.

---

## 3. Docker Infrastructure

**Directory:** `docker/`

### Docker Compose Services (compose/devnet.yaml)

| Service | Image | Role | Ports | Profile |
|---------|-------|------|-------|---------|
| `init-setup` | kora:local | Generate keys only | — | — |
| `init-config` | kora:local | Keys + trusted dealer DKG | — | — |
| `dkg-node{0-3}` | kora:local | Interactive DKG | 30300-3 | interactive-dkg |
| `validator-node{0-3}` | kora:local | Consensus validators | See below | — |
| `secondary-node0` | kora:local | Follower | P2P: 30500 | — |
| `prometheus` | prom/prometheus:latest | Metrics | 9090 | observability |
| `grafana` | grafana/grafana:latest | Dashboards | 3000 | observability |

### Validator Port Mapping

| Node | P2P | RPC | Metrics |
|------|-----|-----|---------|
| validator-node0 | 30400:30303 | 8545:8545 | 9000:9002 |
| validator-node1 | 30401:30303 | 8546:8545 | 9001:9002 |
| validator-node2 | 30402:30303 | 8547:8545 | 9002:9002 |
| validator-node3 | 30403:30303 | 8548:8545 | 9003:9002 |

### Network Topology

```
Docker Bridge Network (kora-net)
├── validator-node0 (bootstrap=true)
│   ├── P2P: 30400 → 30303
│   ├── RPC: 8545 → 8545
│   └── Metrics: 9000 → 9002
├── validator-node1 (bootstrap from node0:30303)
├── validator-node2 (bootstrap from node0:30303)
├── validator-node3 (bootstrap from node0:30303)
├── secondary-node0 (bootstrap from node0:30303)
├── prometheus (scrapes validators on 9002)
└── grafana (queries prometheus)
```

### Validator Common Settings (compose/devnet.yaml)

```yaml
x-validator-common: &validator-common
  image: kora:local
  tmpfs:
    - /runtime:size=1g,mode=1777     # 1GB tmpfs for consensus journal
  healthcheck:
    test: ["CMD", "/scripts/healthcheck.sh"]
    interval: 10s
    timeout: 5s
    retries: 3
    start_period: 30s
  environment:
    - RUST_LOG=${RUST_LOG:-info}
    - CHAIN_ID=${CHAIN_ID:-1337}
    - KORA_RUNTIME_DIR=${KORA_RUNTIME_DIR:-/runtime}
    - HEALTHCHECK_MODE=ready
```

**1GB tmpfs** for `/runtime` ensures consensus journal writes don't hit disk I/O bottlenecks.

### Health Check Logic (scripts/healthcheck.sh)

| Mode | Check |
|------|-------|
| `dkg` | `share.key` AND `output.json` exist |
| `p2p` | Port 30303 is open (netcat) |
| `ready` | `.ready` file exists AND port 30303 open |

### Orchestration Phases (scripts/devnet-run.sh)

1. **Phase 0/3: Build** — `docker buildx bake --load -f docker-bake.hcl kora-local`
2. **Phase 1/3: Configuration** — Check if config exists, run `init-setup` or `init-config`
3. **Phase 1.5/3: DKG** (if interactive) — Start DKG nodes, wait for completion, verify checksums
4. **Phase 2/3: Validators** — Stop existing, clear runtime, start validators + secondary, wait healthy
5. **Phase 3/3: Ready** — Display status table with ports and health

### Startup Modes (scripts/entrypoint.sh)

| Mode | Entrypoint | Requirements |
|------|-----------|--------------|
| `setup` | `keygen setup` | — |
| `dkg` | `kora dkg` | peers.json, validator.key |
| `validator` | `kora validator` | genesis.json, validator.key, share.key, output.json |
| `secondary` | `kora secondary` | peers.json, validator.key |

### Dockerfile Multi-Stage Build

1. **Chef**: rust:nightly + cargo-chef
2. **Planner**: Generate dependency recipe
3. **Builder**: Install deps, cache build, compile `kora` and `keygen`
4. **Runtime**: debian:bookworm-slim + ca-certificates, libssl3, netcat, curl, jq

### Named Volumes

```
data_node{0-3}     - Validator persistent state
data_secondary0    - Secondary peer state
shared_config      - Genesis, peers.json, threshold shares
prometheus_data    - Metrics history
grafana_data       - Dashboard data
```

---

## 4. Issues and Anomalies

| Component | Issue | Severity |
|-----------|-------|----------|
| RPC | Rate limiting configured but not enforced | Medium |
| RPC | No request body size limit set explicitly | Medium |
| RPC | Silent tx drop if tx_submit callback is None | Low (only during misconfiguration) |
| P2P | Resolver blocks peers on "invalid data" | High (blocks catch-up) |
| P2P | Certificate rejection during catch-up | High (prevents sync) |
| Docker | Secondary node has no RPC port exposed | Low (by design) |
| Docker | Grafana uses default admin/admin credentials | Low (dev only) |
| Prometheus | 15s scrape interval (could be 10s for better resolution) | Low |
