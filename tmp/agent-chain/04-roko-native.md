# Roko Native Support — What daeji Must Implement

Roko is an 18-crate, ~177K LOC Rust agent runtime with 5 primitives, 9 protocols,
and a predict-publish-correct learning loop. For roko agents to run natively on
daeji, the chain must implement specific trait interfaces that roko's substrate
layer expects.

---

## Roko's 5 Primitives

| Primitive | What it is | Chain requirement |
|-----------|-----------|-------------------|
| **Signal** | Typed event stream (pub/sub) | eth_subscribe + log events |
| **Pulse** | Heartbeat scheduler (temporal) | Block timestamps + cron-like contract |
| **Cell** | Stateful compute unit | Contract storage + precompile access |
| **Graph** | DAG task executor | Transaction dependency ordering |
| **Protocol** | Interaction pattern | Contract interfaces (ERC-8004, ERC-8183) |

### Signal → daeji Mapping

Roko Signals are typed event streams. On daeji, these map to Solidity events + subscriptions.

```rust
// Roko side
trait SignalBus {
    fn emit(&self, signal: Signal) -> Result<()>;
    fn subscribe(&self, filter: SignalFilter) -> SignalStream;
}

// daeji implementation
struct ChainSignalBus {
    rpc: RpcClient,  // daeji RPC endpoint
}

impl SignalBus for ChainSignalBus {
    fn emit(&self, signal: Signal) -> Result<()> {
        // Encode signal as transaction calling SignalRegistry.emit(...)
        // Submit via eth_sendRawTransaction
        let tx = self.encode_signal_tx(signal);
        self.rpc.send_raw_transaction(tx).await
    }

    fn subscribe(&self, filter: SignalFilter) -> SignalStream {
        // eth_subscribe("logs", { address: SignalRegistry, topics: [...] })
        self.rpc.subscribe_logs(filter.to_log_filter()).await
    }
}
```

### Pulse → daeji Mapping

Roko Pulses are scheduled heartbeats. On daeji, block production IS the heartbeat.

```rust
// Roko side
trait PulseSource {
    fn tick(&self) -> PulseStream;  // emits on interval
}

// daeji implementation: subscribe to newHeads
struct BlockPulse {
    rpc: RpcClient,
}

impl PulseSource for BlockPulse {
    fn tick(&self) -> PulseStream {
        // eth_subscribe("newHeads") → emit pulse per block
        // Block time ~400ms = natural pulse rate
        self.rpc.subscribe_new_heads().await.map(|header| Pulse {
            tick: header.number,
            timestamp: header.timestamp,
        })
    }
}
```

### Cell → daeji Mapping

Roko Cells are stateful compute units. On daeji, each Cell maps to a contract
instance with its own storage.

```rust
// Roko side
trait CellStore {
    fn read(&self, key: &[u8]) -> Result<Option<Vec<u8>>>;
    fn write(&self, key: &[u8], value: &[u8]) -> Result<()>;
}

// daeji implementation: contract storage
struct ContractCell {
    contract_address: Address,
    rpc: RpcClient,
}

impl CellStore for ContractCell {
    fn read(&self, key: &[u8]) -> Result<Option<Vec<u8>>> {
        // eth_getStorageAt(contract, slot_from_key(key), "latest")
        self.rpc.get_storage_at(self.contract_address, key_to_slot(key)).await
    }

    fn write(&self, key: &[u8], value: &[u8]) -> Result<()> {
        // Encode as transaction calling contract.store(key, value)
        let tx = self.encode_store_tx(key, value);
        self.rpc.send_raw_transaction(tx).await
    }
}
```

### Graph → daeji Mapping

Roko's Graph primitive executes DAG-structured task plans. On-chain, this maps to
transaction dependency ordering and batch execution.

The Graph executor runs off-chain (in the roko runtime). It submits completed task
results as transactions. daeji doesn't need special Graph support — it just needs
reliable transaction inclusion and receipt querying.

### Protocol → daeji Mapping

Roko's 9 protocols map to smart contract interfaces:

| Protocol | Contract |
|----------|----------|
| `discover` | AgentRegistry (query capabilities) |
| `negotiate` | JobMarketplace (bid/accept) |
| `delegate` | JobMarketplace (assign/submit) |
| `evaluate` | JobMarketplace (evaluate/dispute) |
| `learn` | InsightBoard (submit/search) |
| `coordinate` | PheromoneRegistry (deposit/scan) |
| `trade` | ClearingHouse (open/close positions) |
| `govern` | GovernanceContract (propose/vote) |
| `attest` | AgentRegistry (TEE attestation) |

---

## Required Trait Implementations

### ChainClient (core RPC access)

```rust
// What roko expects
#[async_trait]
trait ChainClient {
    async fn block_number(&self) -> Result<u64>;
    async fn get_block(&self, number: u64) -> Result<Block>;
    async fn get_balance(&self, address: Address) -> Result<U256>;
    async fn get_code(&self, address: Address) -> Result<Bytes>;
    async fn get_storage_at(&self, address: Address, slot: U256) -> Result<U256>;
    async fn call(&self, tx: TransactionRequest) -> Result<Bytes>;
    async fn estimate_gas(&self, tx: TransactionRequest) -> Result<u64>;
    async fn get_logs(&self, filter: LogFilter) -> Result<Vec<Log>>;
    async fn subscribe_heads(&self) -> Result<HeadStream>;
    async fn subscribe_logs(&self, filter: LogFilter) -> Result<LogStream>;
}

// daeji implementation: thin wrapper over eth_ RPC
struct DaejiChainClient {
    http_url: String,
    ws_url: String,
    provider: Provider,  // alloy or ethers provider
}
```

**Status:** Most methods work today via polling. `subscribe_heads` and `subscribe_logs`
require eth_subscribe implementation.

### ChainWallet (transaction signing + submission)

```rust
#[async_trait]
trait ChainWallet {
    async fn send_transaction(&self, tx: TransactionRequest) -> Result<TxHash>;
    async fn wait_for_receipt(&self, hash: TxHash) -> Result<TransactionReceipt>;
    async fn nonce(&self) -> Result<u64>;
    fn address(&self) -> Address;
}

// daeji implementation: local signer + RPC submission
struct DaejiWallet {
    signer: LocalSigner,  // secp256k1 key
    client: DaejiChainClient,
    nonce_manager: NonceManager,
}
```

**Status:** Fully implementable today. Standard EVM wallet operations.

### HdcSubstrate (HDC operations via chain)

```rust
#[async_trait]
trait HdcSubstrate {
    async fn store(&self, key: H256, vector: HyperVector) -> Result<()>;
    async fn search(&self, query: &HyperVector, top_k: usize) -> Result<Vec<(H256, f32)>>;
    async fn delete(&self, key: H256) -> Result<()>;
    async fn bundle(&self, keys: &[H256]) -> Result<HyperVector>;
    async fn bind(&self, a: H256, b: H256) -> Result<HyperVector>;
}

// daeji implementation: calls to HDC precompile at 0x09
struct DaejiHdcSubstrate {
    wallet: DaejiWallet,
    precompile_address: Address,  // 0x09
}

impl HdcSubstrate for DaejiHdcSubstrate {
    async fn store(&self, key: H256, vector: HyperVector) -> Result<()> {
        // Send tx: call precompile 0x09 with selector 0x01
        let input = encode_store_vector(key, vector);
        self.wallet.send_transaction(TransactionRequest {
            to: self.precompile_address,
            input,
            ..default()
        }).await
    }

    async fn search(&self, query: &HyperVector, top_k: usize) -> Result<Vec<(H256, f32)>> {
        // eth_call to precompile 0x09 with selector 0x02 (read-only, no tx needed)
        let input = encode_search_similar(query, top_k);
        let result = self.wallet.client.call(TransactionRequest {
            to: self.precompile_address,
            input,
            ..default()
        }).await?;
        decode_search_results(result)
    }
}
```

**Status:** Requires HDC precompile. Once the precompile exists, this adapter is trivial.

### ChainSubstrate (high-level chain operations)

```rust
#[async_trait]
trait ChainSubstrate {
    // Identity
    async fn register_agent(&self, code_hash: H256, capabilities: u64) -> Result<U256>;
    async fn get_agent(&self, token_id: U256) -> Result<AgentIdentity>;

    // Knowledge
    async fn submit_insight(&self, kind: InsightKind, content: Bytes, ttl: u64) -> Result<H256>;
    async fn search_insights(&self, query: Bytes, top_k: u8) -> Result<Vec<H256>>;

    // Signaling
    async fn deposit_pheromone(&self, ptype: PheromoneType, location: H256, intensity: u64) -> Result<()>;
    async fn scan_pheromones(&self, location: H256, radius: u64) -> Result<Vec<Pheromone>>;

    // Commerce
    async fn post_job(&self, spec: JobSpec) -> Result<U256>;
    async fn bid_on_job(&self, job_id: U256, price: U256) -> Result<()>;
    async fn submit_work(&self, job_id: U256, deliverable: H256) -> Result<()>;

    // Oracle
    async fn isfr_rate(&self) -> Result<u64>;
    async fn isfr_twap(&self, start: u64, end: u64) -> Result<u64>;
}

// daeji implementation: contract calls via wallet
struct DaejiChainSubstrate {
    wallet: DaejiWallet,
    contracts: ContractAddresses,
}
```

**Status:** Requires all contracts (AgentRegistry, InsightBoard, PheromoneRegistry,
JobMarketplace) + ISFR precompile. Each method is a contract call.

---

## Bus Fabric Integration

Roko's Bus system is a typed pub/sub message fabric. On daeji, Bus channels map to:

| Bus channel | daeji equivalent |
|-------------|-----------------|
| Local Bus (in-process) | Stays in roko runtime, no chain involvement |
| Mesh Bus (P2P) | daeji-chat AEAD rooms (existing) |
| Chain Bus (global store) | Contract events + eth_subscribe |
| Relay Bus (cross-network) | WebSocket relay (see daeji-relay-practical.md in tmp/) |

### Chain Bus Implementation

```
Agent emits to Chain Bus:
  → roko runtime encodes as transaction
  → submits to daeji via eth_sendRawTransaction
  → included in block, event emitted
  → other agents subscribed via eth_subscribe("logs") receive it
```

This is the "global, persistent, ordered" bus — every message is a transaction,
every message has consensus, every message is in the block history.

Expensive but trustworthy. Use for high-value signals only (job postings,
reputation updates, ISFR observations). Use Mesh Bus (daeji-chat) for
high-frequency, low-value coordination.

---

## Feed System Integration

Roko Feeds are continuous data streams (Cell specialization). On daeji:

| Feed type | daeji mapping |
|-----------|--------------|
| Price feeds | ISFR precompile + external oracle contracts |
| Block feeds | eth_subscribe("newHeads") |
| Event feeds | eth_subscribe("logs") with topic filters |
| Pheromone feeds | kora_subscribe("pheromones") |
| Insight feeds | kora_subscribe("insights") |

### Feed Contract Pattern

```solidity
contract Feed {
    event DataPoint(bytes32 indexed feedId, uint64 indexed blockNumber, bytes data);

    mapping(bytes32 => bytes) public latest;
    mapping(bytes32 => mapping(uint64 => bytes)) public history;

    function publish(bytes32 feedId, bytes calldata data) external {
        latest[feedId] = data;
        history[feedId][block.number] = data;
        emit DataPoint(feedId, uint64(block.number), data);
    }
}
```

Agents subscribe to `DataPoint` events filtered by `feedId` to receive specific feeds.

---

## Group System Integration

Roko Groups are persistent agent collectives with 4 coordination modes:

| Mode | On-chain mechanism |
|------|-------------------|
| **Consensus** | Multi-sig or threshold signature on group decisions |
| **Delegation** | Group elects leader, leader submits on behalf |
| **Auction** | Group members bid for task assignment |
| **Stigmergy** | Pheromone-based indirect coordination (no explicit messages) |

### Group Contract

```solidity
contract GroupRegistry {
    struct Group {
        bytes32 groupId;
        address[] members;
        uint8 coordinationMode;  // 0=consensus, 1=delegation, 2=auction, 3=stigmergy
        address leader;          // for delegation mode
        uint64 threshold;        // for consensus mode (e.g., 2/3)
    }

    function createGroup(uint8 mode, address[] calldata members) external returns (bytes32);
    function joinGroup(bytes32 groupId) external;
    function leaveGroup(bytes32 groupId) external;
    function proposeAction(bytes32 groupId, bytes calldata action) external;
    function voteOnAction(bytes32 groupId, uint256 proposalId, bool approve) external;
}
```

---

## Required daeji Changes Summary

### Precompiles (new code in executor)

| Address | Feature | Effort |
|---------|---------|--------|
| `0x09` | HDC vector operations | High |
| `0xA01` | ISFR oracle | Medium-High |
| `0x0B` | QMDB Merkle proofs | Medium |
| `0x0C` | BTLE encryption | Medium |
| `0xA10-0xA1F` | Agent namespace (reserved) | Low (address reservation) |

### Smart Contracts (Solidity, deployed at genesis or via governance)

| Contract | ERC/Spec | Effort |
|----------|----------|--------|
| AgentRegistry | ERC-8004 | Medium |
| JobMarketplace | ERC-8183 | Medium |
| InsightBoard | Custom | Medium |
| PheromoneRegistry | Custom | Medium |
| GroupRegistry | Custom | Low-Medium |
| Feed | Custom | Low |
| SignalRegistry | Custom | Low |
| ReputationRegistry | Custom (7-domain) | Medium |
| ValidationRegistry | Custom | Medium |
| GovernanceContract | Custom | Medium |

### RPC Extensions (new code in node/rpc)

| Method | Category | Effort |
|--------|----------|--------|
| `eth_subscribe("newHeads")` | Standard | Medium |
| `eth_subscribe("logs")` | Standard | Medium |
| `eth_subscribe("newPendingTransactions")` | Standard | Medium |
| `eth_newBlockFilter` | Standard | Low |
| `eth_getBlockReceipts` | Standard | Low |
| `kora_subscribe("pheromones")` | Custom | Low |
| `kora_subscribe("insights")` | Custom | Low |
| `kora_consensusState` | Custom | Medium |

### Roko Adapter Crate (new crate)

A `roko-daeji` adapter crate that implements all roko substrate traits against
daeji's RPC and contracts:

```
roko-daeji/
  src/
    lib.rs
    chain_client.rs       // ChainClient impl
    chain_wallet.rs       // ChainWallet impl
    hdc_substrate.rs      // HdcSubstrate impl
    chain_substrate.rs    // ChainSubstrate impl
    signal_bus.rs         // SignalBus impl (via contract events)
    pulse_source.rs       // PulseSource impl (via newHeads)
    cell_store.rs         // CellStore impl (via contract storage)
    feed_subscriber.rs    // Feed subscription helpers
    group_client.rs       // Group operations
```

This crate is the single integration point. Roko agents import `roko-daeji`
and get a fully functional chain substrate without knowing daeji internals.
