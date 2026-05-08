# 15 -- HDC End-to-End Wiring Guide

> **Status: PARTIALLY OUTDATED -- needs reconciliation with actual codebase**
>
> This guide was written against the original plan (two-crate layout at 0x09).
> The actual implementation has diverged in several ways. The wiring steps
> described here are structurally correct but some details need updating.
>
> **RECONCILIATION NOTES:**
>
> | Aspect | This Guide Says | Actual State | Resolution Needed |
> |--------|----------------|--------------|-------------------|
> | Precompile address | `0x09` | `0x09` in Rust code, but PR #42 discussion mentions `0xA0C` for non-conflicting chain-specific address | Decide canonical address |
> | Crate layout | `kora-hdc` + `kora-hdc-chain` | Both exist as described | No change needed |
> | Precompile crate | `kora-hdc-chain/src/precompile.rs` | Exists AND `crates/node/executor/src/hdc_precompiles.rs` wraps it | Clarify which is the canonical registration point |
> | Opcode table | 0x01-0x06 raw dispatch | Both Rust and Solidity `HdcLib` NOW agree on 0x01-0x06 | Aligned -- no change |
> | Event sync | `FinalizedReporter` + HDC event handler | `event.rs` is STUB ONLY (all topic hashes are `B256::ZERO`, `process_log` is no-op) | Must implement |
> | `HdcConfig` | `crates/node/config/src/hdc.rs` | Exists as described | No change needed |
> | Runner wiring | `.with_hdc()` on `ProductionRunner` | Implemented in `runner.rs` | Working |
> | RPC wiring | `HdcApiImpl` merged into `RpcServer` | Implemented in `crates/node/rpc/src/hdc.rs` | Working |
> | TestApplication | Not addressed | HDC precompile NOT registered in E2E test executor | Must add `.with_hdc_precompile()` |
>
> **Wiring completion checklist:**
>
> - [x] Create `crates/hdc/core/` and `crates/hdc/chain/` crate layout
> - [x] Add workspace members and dependencies
> - [x] Wire `HdcConfig` into `NodeConfig`
> - [x] Wire HDC initialization into `ProductionRunner::run()`
> - [x] Register HDC precompile in `RevmExecutor` (production path)
> - [x] Add `HdcApi` to `RpcServer` module merge
> - [ ] **TODO:** Decide on canonical precompile crate (`kora-hdc-chain/precompile.rs` vs `kora-precompiles`)
> - [ ] **TODO:** Wire event sync from `FinalizedReporter` to HDC index (currently stub)
> - [ ] **TODO:** Add `HdcApi` to `JsonRpcServer` (if separate from `RpcServer`)
> - [ ] **TODO:** Register precompile in `TestApplication` for E2E tests
> - [ ] **TODO:** Implement `fixed_point_decay()` for on-chain consensus-safe decay
> - [ ] **TODO:** Compute real keccak256 topic hashes in `event.rs`

> **Audience.** An agent with zero prior context who needs to understand how
> every HDC piece connects in the daeji/kora codebase.
>
> **Repository root:** `/Users/will/dev/nunchi/daeji/`

---

## 1. Complete Dependency Graph

```
bin/kora                     (binary entry point -- src/main.rs, src/cli.rs)
 |
 +-- kora-runner             (assembles consensus, execution, RPC, ledger)
 |    |
 |    +-- kora-executor      (REVM block execution)
 |    |    |
 |    |    +-- [NEW] kora-hdc-chain   (HDC precompile registered here)
 |    |    |          |
 |    |    |          +-- kora-hdc    (core algebra, no chain deps)
 |    |    |
 |    |    +-- kora-qmdb     (state DB)
 |    |    +-- kora-traits   (StateDb/StateDbRead/StateDbWrite)
 |    |    +-- revm          (EVM engine)
 |    |
 |    +-- kora-rpc           (JSON-RPC server)
 |    |    |
 |    |    +-- [NEW] kora-hdc-chain   (HdcApiImpl, HdcApiServer)
 |    |    |          |
 |    |    |          +-- kora-hdc    (KnowledgeStore, search)
 |    |    |
 |    |    +-- kora-executor
 |    |    +-- kora-indexer
 |    |    +-- kora-traits
 |    |
 |    +-- kora-ledger        (LedgerService, LedgerView)
 |    +-- kora-marshal       (P2P block relay)
 |    +-- kora-reporters     (FinalizedReporter, SeedReporter)
 |    +-- kora-simplex       (consensus engine integration)
 |    +-- kora-consensus     (BlockExecution, SnapshotStore)
 |    +-- kora-txpool        (mempool, TransactionValidator)
 |    +-- kora-indexer       (BlockIndex)
 |    +-- kora-config        (NodeConfig)
 |    +-- kora-service       (NodeRunner trait, NodeRunContext)
 |
 +-- kora-service            (KoraNodeService, LegacyNodeService)
 |    +-- kora-config
 |    +-- kora-transport
 |    +-- daeji-chat
 |
 +-- kora-config             (NodeConfig, ExecutionConfig, etc.)
 +-- kora-dkg                (DKG ceremony)
 +-- kora-rpc                (NodeState)
 +-- kora-transport          (NetworkTransport)
 +-- daeji-chat              (chat layer -- separate from HDC)
```

### New crates introduced by HDC

```
crates/
  hdc/
    core/              <- kora-hdc        (pure algebra + search + knowledge)
    chain/             <- kora-hdc-chain  (precompile + on-chain index + RPC)
```

### Dependency direction (critical -- never invert)

```
kora-hdc  (core algebra, no chain deps)
  ^
  |
kora-hdc-chain  (precompile, on-chain index, event sync)
  ^                    ^
  |                    |
kora-executor        kora-rpc
(registers precompile) (HDC RPC methods)
  ^                    ^
  |                    |
kora-runner (assembles everything)
  ^
  |
kora-service (starts the node)
  ^
  |
bin/kora (binary entry point)
```

---

## 2. Exact File Modifications

### 2.1 Workspace Cargo.toml

**File:** `/Users/will/dev/nunchi/daeji/Cargo.toml`

Add to `[workspace.dependencies]` (after the existing local crate entries):

```toml
# In [workspace.dependencies], add:
kora-hdc = { path = "crates/hdc/core" }
kora-hdc-chain = { path = "crates/hdc/chain" }
```

The workspace `members` glob already covers `crates/hdc/*` because the existing
pattern is:

```toml
members = ["bin/*", "crates/e2e", "crates/network/*", "crates/node/*", "crates/storage/*", "crates/utilities/*"]
```

This does NOT match `crates/hdc/*`. You must either:

**Option A (recommended):** Add an explicit glob:

```toml
members = [
    "bin/*",
    "crates/e2e",
    "crates/hdc/*",       # <-- NEW
    "crates/network/*",
    "crates/node/*",
    "crates/storage/*",
    "crates/utilities/*",
]
```

**Option B:** List explicitly:

```toml
members = [
    "bin/*",
    "crates/e2e",
    "crates/hdc/core",    # <-- NEW
    "crates/hdc/chain",   # <-- NEW
    "crates/network/*",
    "crates/node/*",
    "crates/storage/*",
    "crates/utilities/*",
]
```

### 2.2 kora-hdc Cargo.toml (NEW)

**File:** `crates/hdc/core/Cargo.toml`

```toml
[package]
name = "kora-hdc"
version.workspace = true
edition.workspace = true
rust-version.workspace = true
license.workspace = true
repository.workspace = true
description = "Core HDC algebra, search, and knowledge primitives"

[lints]
workspace = true

[dependencies]
serde.workspace = true
rand.workspace = true
thiserror.workspace = true

# No kora-* or chain dependencies here. Ever.

[dev-dependencies]
rstest.workspace = true
```

### 2.3 kora-hdc-chain Cargo.toml (NEW)

**File:** `crates/hdc/chain/Cargo.toml`

```toml
[package]
name = "kora-hdc-chain"
version.workspace = true
edition.workspace = true
rust-version.workspace = true
license.workspace = true
repository.workspace = true
description = "On-chain HDC integration: precompile, InsightBoard, event sync"

[lints]
workspace = true

[dependencies]
kora-hdc.workspace = true
alloy-primitives.workspace = true
revm.workspace = true
serde.workspace = true
thiserror.workspace = true
tracing.workspace = true

[dev-dependencies]
rstest.workspace = true
```

### 2.4 kora-executor Cargo.toml (MODIFY)

**File:** `crates/node/executor/Cargo.toml`

Currently has:

```toml
[dependencies]
alloy-consensus.workspace = true
alloy-eips.workspace = true
alloy-primitives.workspace = true
alloy-rlp.workspace = true
futures.workspace = true
kora-qmdb = { path = "../../storage/qmdb" }
kora-traits = { path = "../../storage/traits" }
revm.workspace = true
thiserror.workspace = true
tokio = { workspace = true, features = ["rt"] }
```

Add:

```toml
kora-hdc-chain.workspace = true
```

### 2.5 kora-rpc Cargo.toml (MODIFY)

**File:** `crates/node/rpc/Cargo.toml`

Add to `[dependencies]`:

```toml
kora-hdc-chain.workspace = true
```

### 2.6 kora-runner Cargo.toml (MODIFY)

**File:** `crates/node/runner/Cargo.toml`

Add to `[dependencies]`:

```toml
kora-hdc.workspace = true
kora-hdc-chain.workspace = true
```

### 2.7 kora-config (MODIFY)

**File:** `crates/node/config/Cargo.toml` -- add dependency if HdcConfig needs
serde. The current config crate already has `serde.workspace = true`.

**File:** `crates/node/config/src/node.rs` -- add HDC field:

```rust
// Currently (line 17-41):
pub struct NodeConfig {
    pub chain_id: u64,
    pub data_dir: PathBuf,
    pub consensus: ConsensusConfig,
    pub network: NetworkConfig,
    pub execution: ExecutionConfig,
    pub rpc: RpcConfig,
}

// After:
pub struct NodeConfig {
    pub chain_id: u64,
    pub data_dir: PathBuf,
    pub consensus: ConsensusConfig,
    pub network: NetworkConfig,
    pub execution: ExecutionConfig,
    pub rpc: RpcConfig,
    #[serde(default)]
    pub hdc: HdcConfig,              // <-- NEW
}
```

Add a new file `crates/node/config/src/hdc.rs`:

```rust
use std::path::PathBuf;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct HdcConfig {
    /// Enable the HDC precompile at address 0x09.
    #[serde(default = "default_enabled")]
    pub enabled: bool,

    /// Directory for the local knowledge store.
    /// Defaults to `{data_dir}/hdc/knowledge`.
    #[serde(default)]
    pub knowledge_store_path: Option<PathBuf>,

    /// Maximum entries in the local knowledge store.
    #[serde(default = "default_max_entries")]
    pub max_entries: usize,
}

impl Default for HdcConfig {
    fn default() -> Self {
        Self {
            enabled: default_enabled(),
            knowledge_store_path: None,
            max_entries: default_max_entries(),
        }
    }
}

const fn default_enabled() -> bool { false }
const fn default_max_entries() -> usize { 100_000 }
```

Export from `crates/node/config/src/lib.rs`:

```rust
mod hdc;
pub use hdc::HdcConfig;
```

---

## 3. Runtime Wiring -- runner.rs Modifications

**File:** `crates/node/runner/src/runner.rs`

The `ProductionRunner::run()` method (currently lines 202-400) is where all
subsystems are assembled. Here is the modification sequence. Each step shows
WHERE in the existing function to insert code.

### Step 1: Add HDC fields to ProductionRunner

```rust
// In the ProductionRunner struct (line 116-131), add:

/// HDC configuration.
pub hdc_config: Option<kora_config::HdcConfig>,
```

Update `ProductionRunner::new()` to initialize `hdc_config: None`.

Add a builder method:

```rust
#[must_use]
pub fn with_hdc(mut self, hdc_config: kora_config::HdcConfig) -> Self {
    self.hdc_config = Some(hdc_config);
    self
}
```

### Step 2: Initialize HDC system (after QMDB init, before RPC)

In `run()`, after the QMDB `LedgerView::init` call (currently line 220-226)
and the `LedgerService::new` call (line 230), insert:

```rust
// ---- HDC initialization ----
let hdc_components = if let Some(ref hdc_cfg) = self.hdc_config {
    if hdc_cfg.enabled {
        let ks_path = hdc_cfg.knowledge_store_path.clone()
            .unwrap_or_else(|| config.data_dir.join("hdc/knowledge"));
        let knowledge_store = Arc::new(
            kora_hdc::KnowledgeStore::open(&ks_path, hdc_cfg.max_entries)
                .context("init HDC knowledge store")?
        );
        let on_chain_index = Arc::new(
            kora_hdc_chain::OnChainHdcIndex::new()
        );
        info!("HDC system initialized (knowledge_store={}, max_entries={})",
              ks_path.display(), hdc_cfg.max_entries);
        Some((knowledge_store, on_chain_index))
    } else {
        info!("HDC configured but disabled");
        None
    }
} else {
    None
};
```

### Step 3: Wire HDC into RPC (inside the existing RPC block)

In `run()`, the RPC setup block starts at line 233:

```rust
if let Some((node_state, addr)) = &self.rpc_config {
    // ... existing RPC setup ...
```

After the existing `RpcServer` construction (line 270-277) but before `drop(rpc.start())`, add the HDC RPC module:

```rust
// Currently:
let rpc = kora_rpc::RpcServer::with_state_provider(
    node_state.clone(),
    *addr,
    self.chain_id,
    indexed_provider,
)
.with_tx_submit(tx_submit)
.with_peer_count(self.scheme.participants().len().saturating_sub(1) as u64);

// After (add HDC API if enabled):
let rpc = if let Some((ref ks, ref idx)) = hdc_components {
    rpc.with_hdc_api(ks.clone(), idx.clone())  // NEW method on RpcServer
} else {
    rpc
};

drop(rpc.start());
```

### Step 4: Wire HDC precompile into executor

In `run()`, the executor is created at line 287:

```rust
let executor = RevmExecutor::new(self.chain_id);
```

Replace with:

```rust
let executor = if hdc_components.is_some() {
    RevmExecutor::new(self.chain_id)
        .with_hdc_precompile()  // NEW method -- registers 0x09 precompile
} else {
    RevmExecutor::new(self.chain_id)
};
```

A SECOND executor is created at line 344 for the consensus application:

```rust
let executor = RevmExecutor::new(self.chain_id);
```

Apply the same pattern there. Both executors must have the precompile registered
for correctness (one serves RPC calls, one serves block execution).

### Step 5: Wire event sync into FinalizedReporter

After the `FinalizedReporter` is created (line 289-293), add:

```rust
if let Some((ref ks, ref idx)) = hdc_components {
    finalized_reporter = finalized_reporter
        .with_hdc_event_handler(ks.clone(), idx.clone());
}
```

This requires adding HDC event handling to the `FinalizedReporter` in
`crates/node/reporters/`.

---

## 4. Data Flow Diagrams

### 4.1 Transaction Execution with HDC Precompile

```
                    User / Agent
                        |
                        | eth_sendRawTransaction(tx calling 0x09)
                        v
                   +---------+
                   | kora-rpc |
                   |  server  |
                   +----+----+
                        |
                        | tx bytes -> mempool
                        v
                  +-----------+
                  | kora-txpool|
                  |  mempool   |
                  +-----+-----+
                        |
                        | consensus proposes block
                        v
               +----------------+
               | kora-runner    |
               | RevmApplication|
               |  .propose()    |
               +-------+--------+
                        |
                        | execute block txs
                        v
              +-----------------+
              | kora-executor   |
              | RevmExecutor    |
              |  .execute()     |
              +--------+--------+
                        |
                        | REVM hits CALL to 0x09
                        v
            +---------------------+
            | kora-hdc-chain      |
            | precompile handler  |
            +----------+----------+
                        |
                        | decode opcode byte from calldata[0]
                        |
           +------------+------------+
           |            |            |
           v            v            v
     +---------+  +---------+  +-----------+
     | BIND    |  | BUNDLE  |  | SEARCH    |
     | op 0x01 |  | op 0x02 |  | op 0x06  |
     +---------+  +---------+  +-----------+
           |            |            |
           +------------+------------+
                        |
                        | calls kora-hdc core algebra
                        v
              +-----------------+
              | kora-hdc        |
              | bind/bundle/    |
              | permute/hamming |
              +---------+-------+
                        |
                        | result bytes
                        v
              +-----------------+
              | REVM resumes    |
              | gas charged     |
              | output returned |
              +-----------------+
```

### 4.2 RPC Query Flow

```
                   Client
                     |
                     | hdc_search(query_vector, k)
                     v
              +-----------+
              | kora-rpc  |
              | JSON-RPC  |
              | server.rs |
              +-----+-----+
                     |
                     | dispatched to HdcApiImpl
                     v
           +-----------------+
           | kora-hdc-chain  |
           | HdcApiImpl      |
           | .hdc_search()   |
           +--------+--------+
                     |
                     | delegates to local knowledge store
                     v
            +----------------+
            | kora-hdc       |
            | KnowledgeStore |
            | .search(q, k)  |
            +-------+--------+
                     |
                     | LocalIndex / BruteForce / HNSW
                     v
            +----------------+
            | kora-hdc       |
            | search engine  |
            | (tiered)       |
            +-------+--------+
                     |
                     | top-k results
                     v
              +-----------+
              | JSON-RPC  |
              | response  |
              +-----------+
```

### 4.3 Event Sync Loop (Block Finalization)

```
         Consensus finalizes block N
                     |
                     v
          +--------------------+
          | kora-reporters     |
          | FinalizedReporter  |
          | .on_finalized()    |
          +---------+----------+
                     |
                     | iterates over receipts
                     v
         +----------------------+
         | For each receipt log |
         | check topic[0]:     |
         +---+------+----------+
             |      |
             |      |
    topic == |      | topic ==
    InsightPublished  PheromoneDeposited
             |      |
             v      v
   +------------------+  +-------------------+
   | kora-hdc-chain   |  | kora-hdc-chain    |
   | event handler    |  | event handler     |
   | decode log data  |  | decode log data   |
   +--------+---------+  +---------+---------+
             |                      |
             v                      v
   +------------------+  +-------------------+
   | OnChainHdcIndex  |  | OnChainHdcIndex   |
   | .insert_insight()|  | .record_pheromone()|
   +--------+---------+  +---------+---------+
             |                      |
             v                      v
   +------------------+  +-------------------+
   | KnowledgeStore   |  | KnowledgeStore    |
   | .ingest()        |  | (if subscribed)   |
   | (if subscribed)  |  |                   |
   +------------------+  +-------------------+
```

---

## 5. Configuration Schema

### 5.1 NodeConfig (kora-config)

```toml
# Full config file example showing HDC section

chain_id = 1337
data_dir = "/var/lib/kora"

[consensus]
threshold = 3

[network]
listen_addr = "0.0.0.0:9000"

[execution]
gas_limit = 250_000_000
block_time = 50               # milliseconds (after 01-block-time-migration)

[rpc]
http_addr = "0.0.0.0:8546"
ws_addr   = "0.0.0.0:8545"

# NEW -- HDC section
[hdc]
enabled = true                           # default: false
knowledge_store_path = "/var/lib/kora/hdc/knowledge"  # default: {data_dir}/hdc/knowledge
max_entries = 100_000                    # default: 100_000
```

### 5.2 HdcConfig struct (Rust)

```rust
// crates/node/config/src/hdc.rs

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct HdcConfig {
    #[serde(default)]
    pub enabled: bool,

    #[serde(default)]
    pub knowledge_store_path: Option<PathBuf>,

    #[serde(default = "default_max_entries")]
    pub max_entries: usize,
}

// Default: disabled, 100k entries, path derived from data_dir at runtime
```

### 5.3 Where config flows at runtime

```
bin/kora cli.rs
  |
  | Cli::load_config() -> NodeConfig
  | (reads TOML/JSON, applies CLI overrides)
  |
  v
ProductionRunner::new(scheme, chain_id, gas_limit, bootstrap)
    .with_hdc(config.hdc.clone())    <-- passes HdcConfig
    .with_rpc(node_state, rpc_addr)
    .run_standalone(config)
  |
  v
ProductionRunner::run() checks self.hdc_config
  |
  +-- if enabled: init KnowledgeStore, OnChainHdcIndex
  +-- if enabled: register precompile on RevmExecutor
  +-- if enabled: add HdcApi module to RpcServer
  +-- if enabled: add HDC event handler to FinalizedReporter
```

---

## 6. Node Startup Sequence (Step by Step)

This is the full boot sequence in `ProductionRunner::run()` with HDC additions
marked. Reference: `crates/node/runner/src/runner.rs`.

```
 1.  Load validator key from config            (line ~282)
 2.  Build P2P transport                       (done before run() in cli.rs)
 3.  Track validators + secondary peers        (line ~208-215)
 4.  Create page cache                         (line ~217)
 5.  Initialize QMDB ledger (LedgerView)       (line ~220-226)
 6.  Create BlockIndex (for RPC)               (line ~228-229)
 7.  Create LedgerService                      (line ~230)
 8.  Spawn ledger observers                    (line ~231)
 9.  [NEW] Initialize HDC system               <--- if hdc.enabled
       a. Create KnowledgeStore
       b. Create OnChainHdcIndex
       c. Create trust pipeline components
10.  Start RPC server                          (line ~233-279)
       a. Create IndexedStateProvider
       b. Create RevmExecutor for RPC
       c. [NEW] Register HDC precompile on RPC executor
       d. Create TxSubmitCallback
       e. Build RpcServer
       f. [NEW] Attach HdcApi module
       g. rpc.start()
11.  Create RevmExecutor for consensus         (line ~287)
       a. [NEW] Register HDC precompile on consensus executor
12.  Create RevmContextProvider                (line ~288)
13.  Create FinalizedReporter                  (line ~289-293)
       a. [NEW] Attach HDC event handler
14.  Create ConstantSchemeProvider              (line ~295)
15.  Initialize PeerResolver                   (line ~297-303)
16.  Initialize BroadcastEngine                (line ~305-311)
17.  Initialize archive stores                 (line ~315-329)
18.  Initialize marshal actor                  (line ~331-341)
19.  Create epocher + executor + app           (line ~343-353)
20.  Create Inline marshaled                   (line ~354-355)
21.  Create reporters chain                    (line ~357-364)
22.  Submit bootstrap transactions             (line ~366-368)
23.  Create and start simplex engine           (line ~370-396)
24.  Return LedgerService handle               (line ~399)
```

---

## 7. RPC Server Wiring

**File:** `crates/node/rpc/src/server.rs`

The JSON-RPC module assembly happens in the `start()` method (line 237-285).
Currently it merges four API modules:

```rust
// Line 250-275:
let eth_api = ...;
let net_api = ...;
let web3_api = ...;
let kora_api = ...;

let mut module = jsonrpsee::RpcModule::new(());
module.merge(eth_api.into_rpc())?;
module.merge(net_api.into_rpc())?;
module.merge(web3_api.into_rpc())?;
module.merge(kora_api.into_rpc())?;
```

To add HDC:

```rust
// After kora_api merge, add:
if let Some(ref hdc_api) = self.hdc_api {
    if let Err(e) = module.merge(hdc_api.clone().into_rpc()) {
        error!(error = %e, "Failed to merge HDC API");
        return None;
    }
}
```

This requires:
1. Adding an `hdc_api` field to `RpcServer`
2. Adding a `with_hdc_api()` builder method
3. Creating `HdcApiImpl` and `HdcApiServer` in kora-hdc-chain

### HDC RPC methods

The following methods are registered under the `hdc_` namespace:

| Method | Parameters | Returns |
|--------|-----------|---------|
| `hdc_search` | `query: HexVector, k: u32` | `Vec<SearchResult>` |
| `hdc_getInsight` | `id: B256` | `Option<Insight>` |
| `hdc_pheromoneStrength` | `topic: B256, region: B256` | `f64` |
| `hdc_trustScore` | `agent: Address` | `f64` |

---

## 8. Executor Precompile Wiring

**File:** `crates/node/executor/src/revm.rs`

The executor currently builds a plain mainnet EVM:

```rust
// Line 233-234 (simulate_call) and line 383 (execute):
let mut evm = ctx.build_mainnet();
```

To register the HDC precompile, the executor needs a method that replaces
`build_mainnet()` with a builder that includes the custom precompile:

```rust
impl RevmExecutor {
    /// Register the HDC precompile and return self for chaining.
    #[must_use]
    pub fn with_hdc_precompile(mut self) -> Self {
        self.hdc_precompile_enabled = true;
        self
    }
}
```

Then in `execute()` and `simulate_call()`, the EVM construction becomes:

```rust
let mut evm = if self.hdc_precompile_enabled {
    ctx.build_with_precompiles(|precompiles| {
        precompiles.insert(
            kora_hdc_chain::PRECOMPILE_ADDRESS,  // 0x09
            kora_hdc_chain::hdc_precompile(),
        );
    })
} else {
    ctx.build_mainnet()
};
```

The exact REVM API for custom precompiles depends on the REVM version (38.0.0
in this codebase). The precompile is a function with signature:

```rust
fn hdc_precompile(input: &Bytes, gas_limit: u64) -> PrecompileResult
```

See doc `07-precompile-integration.md` for the full precompile specification.

---

## 9. State Persistence

### On-chain (consensus-critical)

HDC vectors stored via InsightBoard contract use `SSTORE` / `SLOAD` opcodes.
The state lives in QMDB just like any other contract storage. This is
consensus-critical -- all validators must agree on the stored values.

### Local knowledge store (node-local)

The `KnowledgeStore` persists to disk using serde/bincode at the path from
`HdcConfig::knowledge_store_path` (defaults to `{data_dir}/hdc/knowledge`).
This is NOT consensus-critical. Each node can have a different view. It is
rebuilt from on-chain events on startup.

### OnChainHdcIndex (node-local, ephemeral)

Rebuilt from event logs on startup by replaying finalized blocks. Lives in
memory. Lost on restart and reconstructed.

```
Startup:
  1. Open KnowledgeStore from disk (or create empty)
  2. Create empty OnChainHdcIndex
  3. Replay finalized block events to populate index
  4. Node is ready to serve HDC queries
```

---

## 10. Anti-Patterns

### DO NOT create circular dependencies

```
WRONG:
  kora-hdc -> kora-hdc-chain    (core depending on chain)

RIGHT:
  kora-hdc-chain -> kora-hdc    (chain depends on core, never the reverse)
```

### DO NOT put chain logic in kora-hdc

The core crate (`kora-hdc`) must have zero knowledge of:
- REVM / precompiles
- Alloy types (Address, B256, etc.)
- On-chain events / logs
- RPC infrastructure
- Consensus / block structure

If you need alloy-primitives in the core algebra, wrap with newtype conversions
in kora-hdc-chain. Keep kora-hdc testable without chain infrastructure.

### DO NOT forget to register the workspace member

If `crates/hdc/core` exists but the workspace `members` glob does not include
`crates/hdc/*`, then `cargo build` will silently skip it and downstream
`workspace = true` references will fail with:

```
error: failed to load manifest for workspace member `/path/to/crates/hdc/core`
```

### DO NOT hardcode paths

Use `HdcConfig::knowledge_store_path` and fall back to `{data_dir}/hdc/knowledge`.
Never hardcode `/var/lib/kora/hdc` or similar.

### DO NOT register the precompile at 0x09 without checking for conflicts

Address `0x09` is not unused: EIP-152 assigns it to the BLAKE2 `F`
compression precompile. REVM's mainnet precompile set includes this address
for Istanbul-and-later spec IDs. If HDC stays at `0x09`, that must be treated
as an intentional sovereign-chain fork choice that replaces BLAKE2F. Otherwise,
move HDC to a non-conflicting chain-specific address such as `0xA0C` and update
`PRECOMPILE_ADDRESS`, `HdcLib.HDC_PRECOMPILE`, deployment docs, RPC examples,
and e2e tests in one change.

### DO NOT add HDC to kora-service

The service crate (`kora-service`) is thin orchestration. HDC wiring belongs in
`kora-runner` (specifically `ProductionRunner::run()`), not in
`LegacyNodeService` or `KoraNodeService`.

### DO NOT create a separate RPC server for HDC

HDC RPC methods are merged into the existing JSON-RPC module in `kora-rpc`.
There is one RPC server, not two. The HDC methods live under the `hdc_`
namespace alongside `eth_`, `net_`, `web3_`, and `kora_` namespaces.

### DO NOT store full vectors in event logs

Event logs (Solidity `emit`) should contain metadata and identifiers, not full
10,240-bit vectors. The full vectors live in contract storage (`SSTORE`). The
event just signals "something changed" so the local index can re-read from
storage or from the precompile.

---

## 10b. Updated Wiring Diagram (Actual State)

The original dependency graph in section 1 shows the PLANNED layout. Here is the
diagram reflecting what ACTUALLY EXISTS in the codebase today:

```
bin/kora                     (binary entry point -- src/main.rs, src/cli.rs)
 |
 +-- kora-runner             (assembles consensus, execution, RPC, ledger)
 |    |
 |    +-- kora-executor      (REVM block execution)
 |    |    |
 |    |    +-- hdc_precompiles.rs   [EXISTS] wraps EthPrecompiles, intercepts 0x09
 |    |    |          |
 |    |    |          +-- kora-hdc-chain::precompile   [EXISTS] raw opcode dispatch
 |    |    |                   |
 |    |    |                   +-- kora-hdc            [EXISTS] core algebra
 |    |    |
 |    |    +-- kora-qmdb     (state DB)
 |    |    +-- kora-traits   (StateDb/StateDbRead/StateDbWrite)
 |    |    +-- revm          (EVM engine)
 |    |
 |    +-- kora-rpc           (JSON-RPC server)
 |    |    |
 |    |    +-- hdc.rs              [EXISTS] HdcApiImpl with 7 RPC methods
 |    |    |          |
 |    |    |          +-- kora-hdc  (KnowledgeStore, search)
 |    |    |
 |    |    +-- kora-executor
 |    |    +-- kora-indexer
 |    |    +-- kora-traits
 |    |
 |    +-- kora-reporters     (FinalizedReporter, SeedReporter)
 |    |    |
 |    |    +-- [STUB] HDC event handler (event.rs -- topics are B256::ZERO, process_log is no-op)
 |    |
 |    +-- kora-config        (NodeConfig, HdcConfig)
 |    +-- (other subsystems unchanged)
 |
 +-- kora-service            (KoraNodeService, LegacyNodeService)
 +-- kora-config             (NodeConfig with hdc field)

crates/
  hdc/
    core/              <- kora-hdc        [EXISTS] algebra + search + knowledge + trust + cognitive
    chain/             <- kora-hdc-chain  [EXISTS] precompile + index + event(stub) + wisdom + rpc

contracts/
  src/
    HdcPrecompile.sol  <- HdcLib library  [EXISTS] opcodes 0x01-0x06, address 0x09
    InsightBoard.sol   <- InsightBoard    [EXISTS] 7-state FSM (uses HdcLib)
```

### What is wired vs what is stubbed

| Component | Wired? | Evidence |
|-----------|--------|----------|
| `ProductionRunner.with_hdc()` | YES | `runner.rs` initializes HDC on startup |
| `RevmExecutor.with_hdc_precompile()` | YES | `hdc_precompiles.rs` intercepts 0x09 |
| `RpcServer` HDC module merge | YES | `server.rs` merges `hdc_api.into_rpc()` |
| `HdcConfig` in `NodeConfig` | YES | `config/src/hdc.rs`, `node.rs` |
| `FinalizedReporter` HDC events | STUB | `event.rs` -- all topic hashes `B256::ZERO`, `process_log` is no-op |
| `OnChainHdcIndex` insert/search | PARTIAL | Index exists, `insert_insight()` works, `record_pheromone()` is TODO |
| `fixed_point_decay()` | MISSING | Not found anywhere in codebase |
| E2E test precompile registration | MISSING | `TestApplication` does not call `.with_hdc_precompile()` |

### Actual node startup sequence (with line references)

```
 1.  Load NodeConfig from TOML/CLI             (cli.rs load_config)
 2.  Extract HdcConfig from NodeConfig         (config.hdc.clone())
 3.  Build ProductionRunner                     (runner.rs)
       .with_hdc(hdc_config)                   [sets self.hdc_config]
       .with_rpc(node_state, rpc_addr)
 4.  ProductionRunner::run()
       4a. Track validators, init QMDB          (lines ~208-230)
       4b. If hdc_config.enabled:
             - Create KnowledgeStore             [DONE]
             - Create OnChainHdcIndex            [DONE]
             - Log "HDC system initialized"      [DONE]
       4c. Start RPC server                      (lines ~233-279)
             - Build RevmExecutor for RPC
             - .with_hdc_precompile()            [DONE]
             - Build RpcServer
             - .with_hdc_api(ks, idx)            [DONE]
             - rpc.start()
       4d. Create RevmExecutor for consensus     (line ~287)
             - .with_hdc_precompile()            [DONE]
       4e. Create FinalizedReporter              (lines ~289-293)
             - .with_hdc_event_handler()         [STUB -- process_log is no-op]
       4f. Create simplex engine, start          (lines ~370-396)
       4g. Return LedgerService handle
```

## 11. Integration Test: Verifying the Wiring

### 11.1 Unit test for precompile registration

```rust
// In crates/node/executor/src/revm.rs #[cfg(test)]

#[test]
fn hdc_precompile_registered() {
    let executor = RevmExecutor::new(1337).with_hdc_precompile();
    assert!(executor.hdc_precompile_enabled);
}
```

### 11.2 Integration test: precompile execution

Add to `crates/e2e/` or a new integration test file. The pattern follows the
existing e2e test structure in `crates/e2e/src/tests/execution.rs`.

```rust
#[tokio::test]
async fn hdc_bind_via_precompile() {
    // 1. Set up a test harness with HDC-enabled executor
    let executor = RevmExecutor::new(1337).with_hdc_precompile();

    // 2. Create two random hypervectors (1280 bytes each)
    let hv_a = random_hypervector();
    let hv_b = random_hypervector();

    // 3. Build calldata: opcode 0x01 (BIND) + hv_a + hv_b
    let mut calldata = vec![0x01u8]; // BIND opcode
    calldata.extend_from_slice(&hv_a);
    calldata.extend_from_slice(&hv_b);

    // 4. Execute a CALL to 0x09 with the calldata
    let result = executor.simulate_call(
        &mock_state,
        CallParams {
            to: Some(Address::from_word(B256::left_padding_from(&[0x09]))),
            data: Bytes::from(calldata),
            gas_limit: Some(100_000),
            ..Default::default()
        },
        &block_context,
    ).unwrap();

    // 5. Verify result is 1280 bytes (one hypervector)
    assert_eq!(result.len(), 1280);

    // 6. Verify result == hv_a XOR hv_b (bind is XOR)
    let expected = xor_vectors(&hv_a, &hv_b);
    assert_eq!(result.as_ref(), expected.as_slice());
}
```

### 11.3 Integration test: RPC method

```rust
#[tokio::test]
async fn hdc_search_rpc() {
    // 1. Start a test node with HDC enabled
    // 2. Insert some knowledge entries via precompile transactions
    // 3. Call hdc_search over JSON-RPC
    // 4. Verify results contain the inserted entries
}
```

### 11.4 Integration test: event sync

```rust
#[tokio::test]
async fn hdc_event_sync_on_finalization() {
    // 1. Start a test node with HDC enabled
    // 2. Submit a transaction that emits InsightPublished
    // 3. Wait for block finalization
    // 4. Verify OnChainHdcIndex was updated
    // 5. Verify KnowledgeStore has the new entry
}
```

### 11.5 Quick smoke test (cargo check)

After all modifications, the minimal verification that everything compiles:

```bash
# From repository root
cargo check --workspace

# Verify the new crates are recognized
cargo check -p kora-hdc
cargo check -p kora-hdc-chain

# Verify the full binary builds
cargo build -p kora
```

---

## 12. File Reference (Quick Lookup)

| What | File |
|------|------|
| Workspace Cargo.toml | `/Users/will/dev/nunchi/daeji/Cargo.toml` |
| Binary entry point | `/Users/will/dev/nunchi/daeji/bin/kora/src/main.rs` |
| CLI (where config is loaded) | `/Users/will/dev/nunchi/daeji/bin/kora/src/cli.rs` |
| Node config struct | `/Users/will/dev/nunchi/daeji/crates/node/config/src/node.rs` |
| Execution config | `/Users/will/dev/nunchi/daeji/crates/node/config/src/execution.rs` |
| ProductionRunner (main wiring) | `/Users/will/dev/nunchi/daeji/crates/node/runner/src/runner.rs` |
| RevmApplication (consensus app) | `/Users/will/dev/nunchi/daeji/crates/node/runner/src/app.rs` |
| Runner lib.rs | `/Users/will/dev/nunchi/daeji/crates/node/runner/src/lib.rs` |
| RevmExecutor | `/Users/will/dev/nunchi/daeji/crates/node/executor/src/revm.rs` |
| Executor lib.rs | `/Users/will/dev/nunchi/daeji/crates/node/executor/src/lib.rs` |
| RPC server assembly | `/Users/will/dev/nunchi/daeji/crates/node/rpc/src/server.rs` |
| RPC lib.rs | `/Users/will/dev/nunchi/daeji/crates/node/rpc/src/lib.rs` |
| Service (LegacyNodeService) | `/Users/will/dev/nunchi/daeji/crates/node/service/src/service.rs` |
| Service lib.rs | `/Users/will/dev/nunchi/daeji/crates/node/service/src/lib.rs` |
| E2E test setup | `/Users/will/dev/nunchi/daeji/crates/e2e/src/setup.rs` |
| E2E test node | `/Users/will/dev/nunchi/daeji/crates/e2e/src/node.rs` |
| E2E execution tests | `/Users/will/dev/nunchi/daeji/crates/e2e/src/tests/execution.rs` |
| HDC core crate (NEW) | `/Users/will/dev/nunchi/daeji/crates/hdc/core/` |
| HDC chain crate (NEW) | `/Users/will/dev/nunchi/daeji/crates/hdc/chain/` |

---

## Audit Findings

> Audit date: 2026-05-08. Auditor compared the spec (sections 1-12 above) against
> the current state of the `hdc` branch. Every claim below references concrete
> file paths and line numbers.

### AF-1: Workspace Cargo.toml -- CORRECT

The workspace `Cargo.toml` (`/Users/will/dev/nunchi/daeji/Cargo.toml`) matches
the spec on all three requirements:

- **Line 2:** `members` glob includes `"crates/hdc/*"` -- workspace member
  registration is correct.
- **Lines 77-78:** `kora-hdc` and `kora-hdc-chain` workspace dependency entries
  are present and point to the correct paths.

No issues.

### AF-2: kora-hdc core crate (Cargo.toml) -- MINOR DEVIATION

**File:** `/Users/will/dev/nunchi/daeji/crates/hdc/core/Cargo.toml`

The spec (section 2.2) specifies dependencies `serde`, `rand`, `thiserror`, and
dev-dependency `rstest`. The implementation has:

- `rand` -- present (matches spec)
- `serde` -- present (matches spec)
- `rand_chacha` -- EXTRA, not in spec (used for deterministic RNG seeding)
- `tiny-keccak` -- EXTRA, not in spec (used for `vector_id` hashing)
- `thiserror` -- MISSING from `[dependencies]` (spec says it should be there)
- `rstest` -- MISSING from `[dev-dependencies]` (replaced by `criterion`)

The core crate correctly has zero `kora-*` chain dependencies, which is the
critical invariant. The extras (`rand_chacha`, `tiny-keccak`) are legitimate
for the core algebra but were not anticipated in the spec. The absence of
`thiserror` means the crate uses ad-hoc error handling or no error enum for
its public API.

### AF-3: kora-hdc-chain crate (Cargo.toml) -- CORRECT + EXTRA

**File:** `/Users/will/dev/nunchi/daeji/crates/hdc/chain/Cargo.toml`

Matches spec section 2.3 exactly, with one addition:

- `parking_lot` -- EXTRA, not in spec (used for `RwLock<OnChainHdcIndex>`
  in the RPC layer). This is a legitimate runtime need that the spec did not
  anticipate.

Dependency direction is correct: `kora-hdc-chain -> kora-hdc` (never inverted).

### AF-4: kora-executor Cargo.toml -- CORRECT

**File:** `/Users/will/dev/nunchi/daeji/crates/node/executor/Cargo.toml`, line 16.

`kora-hdc-chain.workspace = true` is present, matching spec section 2.4.

### AF-5: kora-rpc Cargo.toml -- CORRECT

**File:** `/Users/will/dev/nunchi/daeji/crates/node/rpc/Cargo.toml`, line 45.

`kora-hdc-chain.workspace = true` is present, matching spec section 2.5.
Additionally, `kora-hdc.workspace = true` is in `[dev-dependencies]` (line 54)
for unit tests -- a sensible addition not explicitly in the spec.

### AF-6: kora-runner Cargo.toml -- PARTIAL MATCH

**File:** `/Users/will/dev/nunchi/daeji/crates/node/runner/Cargo.toml`, line 27.

The spec (section 2.6) says both `kora-hdc` and `kora-hdc-chain` should be
added. Implementation has only `kora-hdc-chain` (line 27). `kora-hdc` is
missing. Currently the runner does not directly reference `kora-hdc` types
(it accesses them transitively through `kora-hdc-chain`), so this works at
compile time but violates the explicit spec requirement.

**Impact:** Low. The transitive dependency is sufficient for current usage.
If runner ever needs `kora_hdc::KnowledgeStore::open()` directly (as the spec
section 3 Step 2 envisions), the missing dependency will cause a compile error.

### AF-7: kora-config HdcConfig -- CORRECT

**Files:**
- `/Users/will/dev/nunchi/daeji/crates/node/config/src/hdc.rs`
- `/Users/will/dev/nunchi/daeji/crates/node/config/src/node.rs`, line 44
- `/Users/will/dev/nunchi/daeji/crates/node/config/src/lib.rs`, lines 22-23

All three match the spec (sections 2.7, 5.1, 5.2):
- `HdcConfig` struct has `enabled`, `knowledge_store_path`, `max_entries`
- `NodeConfig` has `#[serde(default)] pub hdc: HdcConfig`
- `lib.rs` exports `HdcConfig`
- Default is `enabled: false`, `max_entries: 100_000`

One minor difference: the spec shows `default_enabled()` as a named serde
default function; the implementation uses `#[serde(default)]` on the bool
field directly (which defaults to `false` via `Default for bool`). Functionally
equivalent.

### AF-8: ProductionRunner HDC wiring -- MOSTLY CORRECT, KEY OMISSIONS

**File:** `/Users/will/dev/nunchi/daeji/crates/node/runner/src/runner.rs`

**8a. Struct field (line 134):** `pub hdc_config: Option<kora_config::HdcConfig>`
-- matches spec Step 1.

**8b. Builder method (lines 175-178):** `with_hdc()` -- matches spec Step 1.

**8c. HDC initialization (lines 231-243):** PARTIAL MATCH.

The spec (Step 2) calls for initializing both `KnowledgeStore` and
`OnChainHdcIndex`. The implementation only creates `OnChainHdcIndex`:

```rust
// Actual (line 236-243):
let hdc_index = if hdc_enabled {
    Some(Arc::new(parking_lot::RwLock::new(
        kora_hdc_chain::OnChainHdcIndex::new(),
    )))
} else {
    None
};
```

**MISSING:** `KnowledgeStore` is not initialized at all in `runner.rs`.
The spec says:

```rust
let knowledge_store = Arc::new(
    kora_hdc::KnowledgeStore::open(&ks_path, hdc_cfg.max_entries)?
);
```

This means:
- The `knowledge_store_path` config field is never read at runtime.
- The `max_entries` config field is never read at runtime.
- The local knowledge store (node-local disk persistence) is not wired.
- The event sync loop cannot feed data into a KnowledgeStore.

**8d. RPC wiring (lines 312-318):** CORRECT in structure.

The `HdcApiImpl` is created from `kora_hdc_chain::rpc::HdcApi::new(idx.clone())`
and attached via `rpc.with_hdc_api(hdc_api)`. This matches spec Step 3.

However, because `KnowledgeStore` is missing, the RPC search only works
against the `OnChainHdcIndex` (in-memory, ephemeral, requires `Accepted`
state). The spec envisions a richer flow where the local `KnowledgeStore`
is also available for `hdc_search`.

**8e. Executor precompile wiring (lines 263-265, 329-331, 389-391):**
CORRECT.

Three executor instances are created in `run()`:
1. RPC executor (line 263) -- `with_hdc_precompile()` applied (line 264-265)
2. FinalizedReporter executor (line 329) -- `with_hdc_precompile()` applied
   (line 330-331)
3. Consensus/app executor (line 389) -- `with_hdc_precompile()` applied
   (line 390-391)

The spec (Step 4) only mentions two executors. The implementation wires
three, which is correct -- the third is used by `RevmApplication` for
block proposal.

**8f. FinalizedReporter HDC event handler (spec Step 5):** NOT IMPLEMENTED.

The spec says:

```rust
finalized_reporter = finalized_reporter
    .with_hdc_event_handler(ks.clone(), idx.clone());
```

Grep for `hdc` in `crates/node/reporters/` returns zero matches. The
`FinalizedReporter` has no `with_hdc_event_handler()` method.
`crates/hdc/chain/src/event.rs` exists with `process_log()` but is never
called from anywhere.

**Impact:** HIGH. The event sync loop described in spec section 4.3 is
entirely disconnected. On-chain InsightPublished/InsightAccepted events
will never populate the `OnChainHdcIndex` or `KnowledgeStore`. The index
will remain permanently empty.

### AF-9: Executor precompile implementation -- CORRECT, WELL-STRUCTURED

**File:** `/Users/will/dev/nunchi/daeji/crates/node/executor/src/hdc_precompiles.rs`

The `HdcPrecompileProvider` wraps `EthPrecompiles` via the REVM
`PrecompileProvider` trait (lines 30-91). This is a clean implementation that:

1. Intercepts calls to `0x09` (line 45)
2. Delegates to `kora_hdc_chain::hdc_precompile()` (line 49)
3. Maps results to REVM `InterpreterResult` (lines 50-77)
4. Falls through to `self.inner.run()` for all other addresses (line 79)
5. Reports `0x09` as warm via `warm_addresses()` (lines 83-87)
6. Reports `0x09` in `contains()` (lines 89-91)

**File:** `/Users/will/dev/nunchi/daeji/crates/node/executor/src/revm.rs`

The `#[path = "hdc_precompiles.rs"] mod hdc_precompiles;` directive (line 32)
is used instead of a standard `mod` path. This works but is unusual.

The `simulate_call()` (lines 273-282) and `execute()` (lines 446-456) methods
both correctly branch on `self.hdc_precompile_enabled` to choose between
`HdcPrecompileProvider` and `ctx.build_mainnet()`.

The macro-based approach (`simulate!` / `execute_block!`) avoids code
duplication between the HDC and non-HDC paths -- this is well done.

### AF-10: RPC server HDC API wiring -- CORRECT

**File:** `/Users/will/dev/nunchi/daeji/crates/node/rpc/src/server.rs`

- `hdc_api: Option<HdcApiImpl>` field on `RpcServer` (line 83)
- `with_hdc_api()` builder method (lines 162-166)
- HDC module merged conditionally in `start()` (lines 291-297)
- All constructor paths initialize `hdc_api: None`

The `HdcApiImpl` in `/Users/will/dev/nunchi/daeji/crates/node/rpc/src/hdc.rs`
is a well-structured jsonrpsee adapter. All 7 methods (`hamming_distance`,
`similarity`, `bind`, `bundle`, `search`, `vector_id`, `encode`) correctly
use `spawn_blocking` for CPU-bound HDC operations, preventing async runtime
starvation.

**Deviation from spec section 7:** The spec lists 4 RPC methods (`hdc_search`,
`hdc_getInsight`, `hdc_pheromoneStrength`, `hdc_trustScore`). The
implementation provides 7 different methods (`hdc_hammingDistance`,
`hdc_similarity`, `hdc_bind`, `hdc_bundle`, `hdc_search`, `hdc_vectorId`,
`hdc_encode`). The spec methods `hdc_getInsight`, `hdc_pheromoneStrength`,
and `hdc_trustScore` are NOT implemented. The implementation provides utility
methods not in the spec.

### AF-11: CLI wiring -- CORRECT

**File:** `/Users/will/dev/nunchi/daeji/bin/kora/src/cli.rs`, lines 189-191.

```rust
if config.hdc.enabled {
    runner = runner.with_hdc(config.hdc.clone());
}
```

This matches spec section 5.3. The config flows from `NodeConfig` through
`ProductionRunner::with_hdc()` into `run()`.

Note: HDC wiring is only in `run_validator()`. The `run_legacy()` and
`run_secondary()` paths do not wire HDC, which is correct -- only validators
need the full HDC subsystem.

### AF-12: Precompile address 0x09 conflict -- ACKNOWLEDGED

**File:** `/Users/will/dev/nunchi/daeji/crates/hdc/chain/src/precompile.rs`,
lines 14-17.

Address `0x09` is the BLAKE2 `F` compression precompile (EIP-152, Istanbul).
The `HdcPrecompileProvider` replaces it for the configured spec ID. The spec
section 10 warns about this. The implementation handles it by completely
overriding the address -- any contract relying on BLAKE2 will break.

This is acceptable for a private chain but should be documented as a known
incompatibility.

### AF-13: Event topic hashes are all B256::ZERO -- STUB

**File:** `/Users/will/dev/nunchi/daeji/crates/hdc/chain/src/event.rs`,
lines 13-25.

All 5 event topic constants (`INSIGHT_PUBLISHED`, `INSIGHT_ACCEPTED`,
`INSIGHT_REJECTED`, `INSIGHT_CHALLENGED`, `PHEROMONE_DEPOSITED`) are set to
`B256::ZERO` with `// TODO: compute actual hash` comments. This means even
if the event processing code were called, it would match on zero-topics and
produce incorrect behavior.

### AF-14: KnowledgeStore::open() signature mismatch -- SPEC DRIFT

**File:** `/Users/will/dev/nunchi/daeji/crates/hdc/core/src/knowledge/store.rs`,
line 33.

The spec (section 3, Step 2) expects:
```rust
KnowledgeStore::open(&ks_path, hdc_cfg.max_entries)
```

The actual signature is:
```rust
pub fn open(_path: &std::path::Path, tick_duration_ms: u64) -> Result<Self, std::io::Error>
```

The second parameter is `tick_duration_ms`, not `max_entries`. The `open()`
method is a stub that ignores its path argument and returns an empty store.
No persistence is implemented.

### AF-15: OnChainHdcIndex search only returns Accepted insights

**File:** `/Users/will/dev/nunchi/daeji/crates/hdc/chain/src/index.rs`,
lines 104-105.

The search method filters: `if meta.state != InsightState::Accepted { return None; }`.
Since events never flow into the index (AF-8f), and `insert_insight()` sets
state to `InsightState::Submitted` (line 82), no inserted insight will ever
appear in search results unless `update_state(..., Accepted)` is called --
which requires the event sync that is not wired.

### AF-16: WisdomGate not wired

**File:** `/Users/will/dev/nunchi/daeji/crates/hdc/chain/src/wisdom.rs`

The `WisdomGate` struct exists with `submit()`, `challenge()`, `resolve()`
methods. It is not referenced from any wiring code -- not from `runner.rs`,
not from the RPC layer, not from event handlers. It is a standalone
implementation with tests but no integration point.

---

## Implementation Status

| Spec Component | Status | File(s) | Notes |
|---|---|---|---|
| Workspace member registration | DONE | `Cargo.toml` line 2 | `crates/hdc/*` glob |
| Workspace dependency entries | DONE | `Cargo.toml` lines 77-78 | Both crates listed |
| kora-hdc core crate | DONE | `crates/hdc/core/` | Full algebra, search, knowledge |
| kora-hdc-chain crate | DONE | `crates/hdc/chain/` | Precompile, index, event stubs |
| HdcConfig struct | DONE | `crates/node/config/src/hdc.rs` | Matches spec |
| NodeConfig.hdc field | DONE | `crates/node/config/src/node.rs` line 44 | `#[serde(default)]` |
| ProductionRunner.hdc_config | DONE | `crates/node/runner/src/runner.rs` line 134 | Option field |
| ProductionRunner.with_hdc() | DONE | `crates/node/runner/src/runner.rs` lines 175-178 | Builder method |
| KnowledgeStore initialization | NOT DONE | runner.rs | Never created in run() |
| OnChainHdcIndex initialization | DONE | runner.rs lines 236-243 | Arc<RwLock<>> wrapped |
| RPC executor precompile | DONE | runner.rs lines 263-265 | Conditional enable |
| Consensus executor precompile | DONE | runner.rs lines 389-391 | Conditional enable |
| FinalizedReporter executor precompile | DONE | runner.rs lines 329-331 | Conditional enable |
| HdcPrecompileProvider | DONE | `executor/src/hdc_precompiles.rs` | Wraps EthPrecompiles |
| Precompile opcodes (0x01-0x06) | DONE | `hdc/chain/src/precompile.rs` | All 6 implemented + tested |
| RPC server hdc_api field | DONE | `rpc/src/server.rs` line 83 | Option<HdcApiImpl> |
| RPC with_hdc_api() builder | DONE | `rpc/src/server.rs` lines 162-166 | Builder pattern |
| RPC HDC module merge | DONE | `rpc/src/server.rs` lines 291-297 | Conditional merge |
| HdcApiImpl (7 methods) | DONE | `rpc/src/hdc.rs` | spawn_blocking for CPU ops |
| HdcRpcApi trait (jsonrpsee) | DONE | `rpc/src/hdc.rs` lines 19-48 | `hdc_` namespace |
| RPC error variants for HDC | DONE | `rpc/src/error.rs` lines 75-88 | 4 HDC error variants |
| CLI config.hdc wiring | DONE | `bin/kora/src/cli.rs` lines 189-191 | Conditional enable |
| FinalizedReporter event handler | NOT DONE | `crates/node/reporters/` | No HDC code exists |
| Event topic hashes | STUB | `hdc/chain/src/event.rs` lines 13-25 | All B256::ZERO |
| Event log processing | STUB | `hdc/chain/src/event.rs` line 29 | Body is TODO comments |
| KnowledgeStore persistence | STUB | `hdc/core/src/knowledge/store.rs` line 33 | open() ignores path |
| WisdomGate integration | NOT DONE | `hdc/chain/src/wisdom.rs` | No wiring in runner/RPC |
| hdc_getInsight RPC method | NOT DONE | -- | Spec method not implemented |
| hdc_pheromoneStrength RPC method | NOT DONE | -- | Spec method not implemented |
| hdc_trustScore RPC method | NOT DONE | -- | Spec method not implemented |
| On-chain index replay on startup | NOT DONE | -- | Spec section 9 startup step 3 |
| E2E integration tests | PARTIAL | `crates/e2e/src/tests/hdc.rs` | File exists (untracked) |

---

## Anti-Patterns & Duct Tape

### DT-1: `#[path = "..."]` module declaration

**File:** `/Users/will/dev/nunchi/daeji/crates/node/executor/src/revm.rs`,
lines 32-34.

```rust
#[path = "hdc_precompiles.rs"]
mod hdc_precompiles;
use hdc_precompiles::HdcPrecompileProvider;
```

Using `#[path]` is non-standard. The standard approach is to place the file
at `src/hdc_precompiles.rs` and declare `mod hdc_precompiles;` without the
attribute. The file IS already at the correct path for the standard approach.
The `#[path]` attribute is unnecessary and confusing.

**Severity:** Low (cosmetic).

### DT-2: `parking_lot::RwLock` managed directly in runner

**File:** `/Users/will/dev/nunchi/daeji/crates/node/runner/src/runner.rs`,
lines 238-240.

```rust
Some(Arc::new(parking_lot::RwLock::new(
    kora_hdc_chain::OnChainHdcIndex::new(),
)))
```

The runner directly constructs the `Arc<RwLock<OnChainHdcIndex>>` and passes
it to the RPC layer. This tightly couples the runner to the concurrency
primitive choice. A factory method on `OnChainHdcIndex` (e.g.,
`OnChainHdcIndex::new_shared()`) or a wrapper type in `kora-hdc-chain`
would be cleaner.

**Severity:** Low (coupling).

### DT-3: Three near-identical executor creation blocks

**File:** `/Users/will/dev/nunchi/daeji/crates/node/runner/src/runner.rs`,
lines 263-266, 329-332, 389-392.

Each block follows the same pattern:

```rust
let mut executor = RevmExecutor::new(self.chain_id);
if hdc_enabled {
    executor = executor.with_hdc_precompile();
}
```

This is repeated three times. A helper method like
`self.make_executor() -> RevmExecutor` would eliminate duplication and
ensure all three executors are always configured identically.

**Severity:** Medium (DRY violation, risk of config drift between executors).

### DT-4: Event topic hashes hardcoded to B256::ZERO

**File:** `/Users/will/dev/nunchi/daeji/crates/hdc/chain/src/event.rs`,
lines 13-25.

All five event topic constants are `B256::ZERO`. This is duct tape: the
values should be computed from the Solidity event signatures via keccak256.
Using `B256::ZERO` means every topic-zero log (which is common in non-event
log entries) would incorrectly match these event handlers if they were ever
called.

**Severity:** HIGH (correctness bug if event processing is ever activated).

### DT-5: `process_log()` body is entirely TODO comments

**File:** `/Users/will/dev/nunchi/daeji/crates/hdc/chain/src/event.rs`,
lines 28-54.

The function signature exists but the body only contains `info!()` log
statements and comments like `// Decode: vector_id, publisher, block_number
from log data`. No actual ABI decoding or index mutation occurs.

**Severity:** HIGH (the event sync pipeline is a no-op).

### DT-6: KnowledgeStore::open() is a fake constructor

**File:** `/Users/will/dev/nunchi/daeji/crates/hdc/core/src/knowledge/store.rs`,
lines 33-35.

```rust
pub fn open(_path: &std::path::Path, tick_duration_ms: u64) -> Result<Self, std::io::Error> {
    Ok(Self::new(tick_duration_ms))
}
```

The `_path` parameter is prefixed with underscore (intentionally unused).
This means:
- No persistence to disk
- No recovery from disk on restart
- The `knowledge_store_path` config field is dead code

The second parameter is `tick_duration_ms` but the spec and runner expect
it to be `max_entries`. This is an API mismatch.

**Severity:** HIGH (persistence is advertised but not implemented).

### DT-7: Macro-based EVM dispatch in executor

**File:** `/Users/will/dev/nunchi/daeji/crates/node/executor/src/revm.rs`,
lines 252-271 (`simulate!` macro) and lines 413-444 (`execute_block!` macro).

Rust macros are used to avoid duplicating the EVM dispatch logic for the
HDC vs non-HDC code path. While this works, it makes the code harder to
read, debug (stack traces point to the macro invocation), and test. An
alternative would be to use a generic function parameterized on the
precompile provider, or to always use `HdcPrecompileProvider` when HDC is
compiled in (it falls through to standard precompiles for non-0x09
addresses anyway).

**Severity:** Low (readability).

### DT-8: `record_pheromone()` is a no-op

**File:** `/Users/will/dev/nunchi/daeji/crates/hdc/chain/src/index.rs`,
lines 130-132.

```rust
pub fn record_pheromone(&mut self, _topic: B256, _region: B256, _strength: u64) {
    // TODO: implement pheromone tracking
}
```

All three parameters are unused. The pheromone system from the spec
(section 4.3) has no backing implementation.

**Severity:** Medium (feature gap).

### DT-9: `JsonRpcServer` does not support HDC API

**File:** `/Users/will/dev/nunchi/daeji/crates/node/rpc/src/server.rs`,
lines 344-437.

The `JsonRpcServer` struct (used for standalone JSON-RPC without HTTP status)
has no `hdc_api` field and no `with_hdc_api()` builder. If any code path
uses `JsonRpcServer` instead of `RpcServer`, HDC methods would silently not
be registered. Currently only `RpcServer` is used in production, so this is
low risk.

**Severity:** Low (dead path, but inconsistent API surface).

### DT-10: `kora-hdc` missing from runner Cargo.toml

**File:** `/Users/will/dev/nunchi/daeji/crates/node/runner/Cargo.toml`

Only `kora-hdc-chain` is listed (line 27). `kora-hdc` is not a direct
dependency. The spec (section 2.6) says both should be present. Currently
the runner accesses `kora-hdc` types transitively, which works but is
fragile -- a refactor in `kora-hdc-chain` that stops re-exporting a
`kora-hdc` type would break the runner.

**Severity:** Low (build fragility).

---

## Recommended Changes Checklist

### Priority 1 -- Critical (blocks end-to-end HDC functionality)

- [ ] **Wire FinalizedReporter HDC event handler.** Add a
  `with_hdc_event_handler()` method to `FinalizedReporter` in
  `crates/node/reporters/`. In `runner.rs`, call it after constructing
  `finalized_reporter` (line 338). This should iterate receipt logs, match
  against HDC event topics, and call `process_log()` from
  `kora_hdc_chain::event`.
  - Files: `crates/node/reporters/src/finalized.rs`, `runner.rs` ~line 338

- [ ] **Compute real event topic hashes.** Replace `B256::ZERO` in
  `crates/hdc/chain/src/event.rs` lines 13-25 with actual keccak256 hashes
  of the Solidity event signatures. Use `alloy_primitives::keccak256()` as
  a const or lazy_static.
  - File: `crates/hdc/chain/src/event.rs`

- [ ] **Implement `process_log()` body.** ABI-decode log data for each event
  type and call the appropriate `OnChainHdcIndex` mutation methods
  (`insert_insight`, `update_state`, `record_pheromone`).
  - File: `crates/hdc/chain/src/event.rs` lines 28-54

### Priority 2 -- High (needed for spec-complete behavior)

- [ ] **Initialize KnowledgeStore in runner.** After OnChainHdcIndex creation
  (line 243), create a `KnowledgeStore` using the config's
  `knowledge_store_path` and `max_entries`. Pass it alongside `hdc_index`
  to the RPC layer and event handler.
  - File: `crates/node/runner/src/runner.rs` ~line 243

- [ ] **Fix KnowledgeStore::open() signature.** Change second parameter from
  `tick_duration_ms: u64` to `max_entries: usize`, or add a separate
  parameter. Implement actual disk persistence (even if initially simple
  serde/bincode).
  - File: `crates/hdc/core/src/knowledge/store.rs` line 33

- [ ] **Implement spec RPC methods.** Add `hdc_getInsight`,
  `hdc_pheromoneStrength`, `hdc_trustScore` to the `HdcRpcApi` trait and
  `HdcApiImpl`.
  - Files: `crates/node/rpc/src/hdc.rs`, `crates/hdc/chain/src/rpc.rs`

- [ ] **Wire WisdomGate.** Integrate `WisdomGate` lifecycle (submit, challenge,
  resolve) into the event processing pipeline and expose via RPC.
  - Files: `crates/hdc/chain/src/wisdom.rs`, `crates/hdc/chain/src/event.rs`

### Priority 3 -- Medium (code quality, robustness)

- [ ] **Extract executor factory method.** Replace the three identical
  executor-construction blocks in `runner.rs` with a helper:
  ```rust
  fn make_executor(&self) -> RevmExecutor {
      let mut e = RevmExecutor::new(self.chain_id);
      if self.hdc_config.as_ref().is_some_and(|c| c.enabled) {
          e = e.with_hdc_precompile();
      }
      e
  }
  ```
  - File: `crates/node/runner/src/runner.rs`

- [ ] **Implement `record_pheromone()`.** Fill in the pheromone tracking body
  in `OnChainHdcIndex`.
  - File: `crates/hdc/chain/src/index.rs` lines 130-132

- [ ] **Add `kora-hdc` to runner Cargo.toml.** Per spec section 2.6, add
  `kora-hdc.workspace = true` as a direct dependency.
  - File: `crates/node/runner/Cargo.toml`

- [ ] **Implement on-chain index replay on startup.** After initializing
  `OnChainHdcIndex`, replay finalized block events to populate it. This is
  spec section 9 startup step 3.
  - File: `crates/node/runner/src/runner.rs`

### Priority 4 -- Low (cosmetic, minor)

- [ ] **Remove `#[path]` attribute.** Change `#[path = "hdc_precompiles.rs"]`
  to a bare `mod hdc_precompiles;` in `crates/node/executor/src/revm.rs`
  line 32.

- [ ] **Add HDC API support to `JsonRpcServer`.** Add `hdc_api` field and
  `with_hdc_api()` to `JsonRpcServer` for API surface consistency.
  - File: `crates/node/rpc/src/server.rs`

- [ ] **Add `thiserror` to kora-hdc core dependencies.** Per spec section 2.2.
  - File: `crates/hdc/core/Cargo.toml`

- [ ] **Consider always using HdcPrecompileProvider.** Since it falls through
  to `EthPrecompiles` for non-0x09 addresses, the `hdc_precompile_enabled`
  flag and the macro-based dispatch could be eliminated. The provider would
  be a no-op for the 0x09 path when disabled (return error or empty output).
  This would remove the macros in `revm.rs` and simplify the code.
  - Files: `crates/node/executor/src/revm.rs`,
    `crates/node/executor/src/hdc_precompiles.rs`

---

## Second-Pass Remediation Detail

> Second pass date: 2026-05-08. This section supersedes the high-level
> checklist with a concrete wiring plan based on the current runner,
> executor, RPC, config, reporter, and HDC chain code.

### Current end-to-end state

The current HDC path is partially wired:

```
config.hdc.enabled
  -> bin/kora/src/cli.rs:189-191
  -> ProductionRunner::with_hdc(config.hdc.clone())
  -> runner.rs:232-243 creates Arc<RwLock<OnChainHdcIndex>>
  -> runner.rs:263-266, 329-332, 389-392 enables RevmExecutor HDC precompile
  -> runner.rs:312-318 registers kora_rpc::HdcApiImpl
  -> rpc/server.rs:291-297 merges hdc_* methods
  -> rpc/hdc.rs -> hdc/chain/rpc.rs -> OnChainHdcIndex::search()
```

The missing end-to-end path is:

```
finalized block receipts/logs
  -> FinalizedReporter finalization hook
  -> kora_hdc_chain::event::process_log(...)
  -> OnChainHdcIndex::insert_insight/update_state/record_pheromone
  -> KnowledgeStore::insert/search persistence layer
  -> hdc_search / hdc_getInsight / hdc_pheromoneStrength / hdc_trustScore
```

Until that path exists, HDC RPC search remains connected to an empty
node-local index. `OnChainHdcIndex::search()` filters to `InsightState::Accepted`,
`insert_insight()` starts at `Submitted`, and no finalized event currently moves
an insight to `Accepted`.

### Target wiring shape

Introduce one runner-owned shared component bundle and pass clones to the three
integration points.

```rust
#[derive(Clone)]
struct HdcRuntime {
    config: kora_config::HdcConfig,
    index: Arc<parking_lot::RwLock<kora_hdc_chain::OnChainHdcIndex>>,
    knowledge: Arc<parking_lot::RwLock<kora_hdc::KnowledgeStore>>,
}
```

The runner should construct this once in `ProductionRunner::run()` and store it
in a local `Option<HdcRuntime>`:

```rust
let hdc = self
    .hdc_config
    .as_ref()
    .filter(|cfg| cfg.enabled)
    .map(|cfg| init_hdc_runtime(cfg, &config.data_dir, self.block_time_ms))
    .transpose()?;
```

Use `block_time_ms` as the current `KnowledgeStore::open(path, tick_duration_ms)`
argument until the store API is corrected. Do not pass `max_entries` to the
current `open()` signature; that would compile only after changing
`crates/hdc/core/src/knowledge/store.rs`.

Required direct dependencies for this target shape:

- `crates/node/runner/Cargo.toml`: add `kora-hdc.workspace = true` because the
  runner will directly construct `kora_hdc::KnowledgeStore`.
- `crates/node/reporters/Cargo.toml`: add `kora-hdc.workspace = true`,
  `kora-hdc-chain.workspace = true`, and `parking_lot.workspace = true` for the
  finalization hook.
- `crates/node/rpc/Cargo.toml`: keep `kora-hdc-chain.workspace = true`; add
  `kora-hdc.workspace = true` to `[dependencies]` if the RPC API is extended to
  read the `KnowledgeStore` directly instead of only through chain-level `HdcApi`.

### Initialization order

The correct boot order in `ProductionRunner::run()` should be:

1. Extract `(context, config, transport)` and register primary/secondary peers.
2. Initialize QMDB via `LedgerView::init(...)`.
3. Create `BlockIndex` if RPC is enabled.
4. Create `LedgerService` and spawn ledger observers.
5. Initialize HDC runtime if `self.hdc_config.enabled`:
   - Resolve `knowledge_store_path` to `cfg.knowledge_store_path` or
     `config.data_dir.join("hdc/knowledge")`.
   - Open `KnowledgeStore` with `tick_duration_ms = self.block_time_ms` for the
     current API.
   - Create `OnChainHdcIndex::new()` inside `Arc<parking_lot::RwLock<_>>`.
   - Log `enabled`, `knowledge_store_path`, `max_entries`, and `tick_duration_ms`.
6. Optionally replay already-finalized blocks into the HDC runtime before RPC
   starts. If replay support is not available yet, explicitly log that HDC
   replay is skipped and RPC search starts cold.
7. Start RPC and attach HDC API from the same `HdcRuntime`.
8. Build the finalized reporter with an HDC-enabled executor and attach the HDC
   finalization hook.
9. Build the consensus application with an HDC-enabled executor.
10. Start marshal, reporters, bootstrap transaction submission, and simplex.

HDC should initialize after `LedgerService` exists because startup replay and
future event sync need finalized block data/receipts. RPC should start after
HDC runtime creation so the API never exposes a method namespace backed by a
missing index.

### Event sync finalization hook

`crates/node/reporters/src/lib.rs` already re-executes finalized blocks in
`handle_finalized_update()` when a snapshot is missing or when RPC indexing is
enabled. That function has the receipts needed for HDC sync in
`execution_outcome`. Extend the reporter with an optional HDC hook and process
logs after root validation and before `ack.acknowledge()`.

Required reporter fields:

```rust
struct HdcFinalizationSink {
    index: Arc<parking_lot::RwLock<kora_hdc_chain::OnChainHdcIndex>>,
    knowledge: Arc<parking_lot::RwLock<kora_hdc::KnowledgeStore>>,
}

pub struct FinalizedReporter<E, P> {
    ...
    hdc_sink: Option<HdcFinalizationSink>,
}
```

Required builder:

```rust
#[must_use]
pub fn with_hdc_event_handler(
    mut self,
    index: Arc<parking_lot::RwLock<kora_hdc_chain::OnChainHdcIndex>>,
    knowledge: Arc<parking_lot::RwLock<kora_hdc::KnowledgeStore>>,
) -> Self {
    self.hdc_sink = Some(HdcFinalizationSink { index, knowledge });
    self
}
```

Required call site in `runner.rs` immediately after `with_block_index(...)`:

```rust
if let Some(hdc) = &hdc {
    finalized_reporter =
        finalized_reporter.with_hdc_event_handler(hdc.index.clone(), hdc.knowledge.clone());
}
```

Required hook inside `handle_finalized_update()`:

```rust
if let (Some(sink), Some(outcome)) = (hdc_sink.as_ref(), execution_outcome.as_ref()) {
    sync_hdc_from_finalized_receipts(sink, block.height, outcome);
}
```

The hook should iterate successful receipts only, then each log:

```rust
for receipt in outcome.receipts.iter().filter(|r| r.success()) {
    for log in receipt.logs() {
        let Some(topic0) = log.topics().first() else { continue; };
        let mut index = sink.index.write();
        kora_hdc_chain::event::process_log(
            &mut index,
            topic0,
            log.data.data.as_ref(),
            &log.address,
        );
    }
}
```

If `process_log()` is extended to also hydrate `KnowledgeStore`, prefer a single
chain-level function that receives both stores and hides the locking order:

```rust
kora_hdc_chain::event::process_finalized_log(
    &mut index,
    &mut knowledge,
    block_height,
    &log.address,
    log.data.topics(),
    log.data.data.as_ref(),
)?;
```

This avoids future deadlocks by enforcing one lock order: index write first,
knowledge write second. The function should return `Result<(), HdcEventError>`
and the reporter should log and continue on malformed non-consensus local-index
events, not reject or withhold finalized-block acknowledgements.

### Event processor requirements

Before wiring the reporter hook, `crates/hdc/chain/src/event.rs` must stop being
a no-op:

- Replace all `B256::ZERO` topic constants with keccak256 hashes of the exact
  Solidity event signatures.
- Decode event data and indexed topics according to the contract ABI, not by
  ad hoc slicing.
- For `InsightPublished`, insert the vector and metadata. If the event only
  carries `vector_id`, fetch the full vector from contract storage during replay
  or change the event/contract interface to include enough data to hydrate the
  local index.
- For `InsightAccepted`, call `update_state(&id, InsightState::Accepted)` and
  insert/hydrate the corresponding `KnowledgeEntry` into `KnowledgeStore`.
- For `InsightRejected` and `InsightChallenged`, update state and avoid adding
  rejected/challenged entries to search-visible knowledge.
- For `PheromoneDeposited`, implement `OnChainHdcIndex::record_pheromone(...)`
  and expose read access for RPC.

Important invariant: the HDC local index and knowledge store are node-local
derived state. Failure to decode one HDC event should not alter consensus state,
but it must be visible in logs/metrics and tests.

### Executor construction consolidation

`ProductionRunner::run()` currently has three near-identical executor creation
blocks:

- RPC read-only executor for `eth_call` / `estimateGas`.
- FinalizedReporter replay executor.
- Consensus application executor.

Consolidate this into one helper on `ProductionRunner`:

```rust
fn hdc_enabled(&self) -> bool {
    self.hdc_config.as_ref().is_some_and(|cfg| cfg.enabled)
}

fn make_executor(&self) -> RevmExecutor {
    let executor = RevmExecutor::new(self.chain_id);
    if self.hdc_enabled() {
        executor.with_hdc_precompile()
    } else {
        executor
    }
}
```

Then use:

```rust
let rpc_executor = Arc::new(self.make_executor());
let finalized_executor = self.make_executor();
let app_executor = self.make_executor();
```

This is not just style. If one of the three executors loses
`with_hdc_precompile()`, behavior diverges between `eth_call`, finalized replay,
and proposal/verification execution. That can make RPC appear to support HDC
while finalized blocks or proposals do not.

The same consolidation is needed in the e2e harness. `crates/e2e/src/harness.rs`
currently constructs `RevmExecutor::new(chain_id)` for `FinalizedReporter` and
`RevmExecutor::new(1337)` inside `TestApplication::new()`. HDC e2e tests must
either enable `with_hdc_precompile()` in both places or add a `TestConfig`
boolean such as `hdc_enabled` and thread it into both executors.

### Config/RPC/precompile dependency contract

Config contract:

- `HdcConfig.enabled` gates all HDC runtime wiring.
- `knowledge_store_path` must be read at runtime; today it is not.
- `max_entries` must either be enforced by `KnowledgeStore` or removed/renamed.
  The current `KnowledgeStore::open(path, tick_duration_ms)` signature means
  `max_entries` is not connected.
- If `max_entries` remains in config, change store construction to an explicit
  options object to avoid positional-argument drift:

```rust
pub struct KnowledgeStoreOptions {
    pub tick_duration_ms: u64,
    pub max_entries: usize,
}
```

RPC contract:

- `RpcServer` already supports optional `HdcApiImpl`; keep HDC merged into the
  existing JSON-RPC server, not a second server.
- Chain-level `HdcApi` currently owns only
  `Arc<RwLock<OnChainHdcIndex>>`. To satisfy the original API surface, extend it
  to receive `Arc<RwLock<KnowledgeStore>>` as well.
- Add spec methods if they remain required:
  `hdc_getInsight`, `hdc_pheromoneStrength`, and `hdc_trustScore`.
- Keep CPU-heavy vector work inside `spawn_blocking` in `crates/node/rpc/src/hdc.rs`.

Precompile contract:

- `HdcPrecompileProvider` correctly intercepts `PRECOMPILE_ADDRESS` and delegates
  to standard `EthPrecompiles` otherwise.
- `PRECOMPILE_ADDRESS` is `0x09`, replacing the Ethereum BLAKE2 precompile for
  this private chain. Keep that documented as an incompatibility.
- Precompile output is not automatically indexed. Only logs from InsightBoard or
  related contracts should feed `OnChainHdcIndex` / `KnowledgeStore`.

### Tests required for remediation

Add focused unit tests before e2e tests:

- `kora-hdc-chain::event`: topic constants are non-zero and equal to known
  keccak256 values for the contract event signatures.
- `kora-hdc-chain::event`: each supported log decodes into the expected
  `OnChainHdcIndex` mutation.
- `OnChainHdcIndex`: `InsightPublished` followed by `InsightAccepted` makes an
  insight search-visible; rejected/challenged insights remain hidden.
- `kora-reporters`: a fabricated `ExecutionOutcome` with one successful receipt
  and one HDC log calls the HDC finalization sink exactly once. A failed receipt
  must not mutate HDC state.
- `runner`: `make_executor()` returns an executor with
  `hdc_precompile_enabled == true` only when `HdcConfig.enabled` is true.

Then add integration tests:

- RPC smoke: start `RpcServer` with `with_hdc_api(...)` and assert
  `hdc_hammingDistance`, `hdc_bind`, `hdc_vectorId`, and `hdc_search` are
  registered and callable.
- Finalization sync: execute or fabricate a finalized block containing an HDC
  event log, let `FinalizedReporter` process it, and assert the shared
  `OnChainHdcIndex` contains the vector in `Accepted` state.
- Knowledge store wiring: enable HDC with an explicit `knowledge_store_path`,
  finalize an accepted insight, and assert the shared `KnowledgeStore` receives
  the entry. When persistence is implemented, restart/open the store and assert
  the entry survives.
- E2E precompile: update `TestConfig` with `with_hdc_enabled()` and ensure both
  the test application executor and finalized reporter executor use
  `with_hdc_precompile()`. The existing `crates/e2e/src/tests/hdc.rs` tests
  should opt in explicitly so they prove HDC is wired instead of merely proving
  blocks still finalize.
- E2E event-to-RPC: submit an InsightBoard transaction that emits
  `InsightPublished` and `InsightAccepted`, wait for finalization, then call
  `hdc_search` and assert the inserted vector ID is returned.

Minimal verification commands after implementation:

```bash
cargo test -p kora-hdc-chain event
cargo test -p kora-reporters hdc
cargo test -p kora-runner hdc
cargo test -p kora-rpc hdc
cargo nextest run -p kora-e2e --test-threads=1 hdc
cargo clippy --all-targets --all-features -- -D warnings
```
