# Implement functional secondary (follower) node mode

## Summary

The `kora secondary` CLI subcommand exists and appears to run a secondary node, but it is entirely a stub. After joining the P2P network, it calls `futures::future::pending::<()>().await`, which blocks forever without doing any useful work. No blocks are received, no state is built, no RPC server is started, and no metrics are exported. The secondary node currently contributes nothing beyond occupying a slot in the Commonware peer mesh.

A functional secondary node (also called a "follower" or "full node") would receive finalized blocks from validators via the marshal actor stack, re-execute them to build local state, and serve read-only Ethereum JSON-RPC queries. This is essential for RPC load distribution, block explorers, indexers, and archive node use cases.

## Current behavior

### The stub implementation

In `bin/kora/src/cli.rs` (lines 194-246), the `run_secondary` method:

1. Loads identity key and `peers.json`
2. Builds a Commonware P2P transport with 5 channels (votes, certs, resolver, blocks, backfill)
3. Registers tracked peers (both validators and secondaries) with the discovery oracle at epoch 0
4. Logs `"secondary peer joined network"`
5. Calls `futures::future::pending::<()>().await` -- **hangs forever**

```rust
// bin/kora/src/cli.rs:242 -- the critical line
futures::future::pending::<()>().await;
```

### What is missing

| Capability | Status |
|---|---|
| Receive finalized blocks via marshal actor | Not implemented |
| Verify finalization certificates (BLS threshold signatures) | Not implemented |
| Execute blocks to build local state (QMDB) | Not implemented |
| Serve JSON-RPC (eth_blockNumber, eth_getBalance, eth_call, etc.) | Not implemented |
| Export Prometheus metrics | Not implemented |
| Load genesis state | Not implemented -- genesis.json is not copied into the secondary data dir |
| Block index for receipts/logs/transactions | Not implemented |
| Transaction relay (forward eth_sendRawTransaction to validators) | Not implemented |
| Catch-up sync for historical blocks on first start | Not implemented |
| Healthcheck beyond P2P port | Not implemented |

### CLI argument gaps

The `SecondaryArgs` struct accepts only `--peers`:

```rust
// bin/kora/src/cli.rs:58-63
#[derive(clap::Args, Debug)]
pub(crate) struct SecondaryArgs {
    /// Path to peers.json file containing primary and secondary peer information.
    #[arg(long)]
    pub peers: PathBuf,
    // Missing: --metrics-addr, --rpc-addr, --genesis
}
```

Compare with `ValidatorArgs` which also has `--metrics-addr` and `--peers`.

### Docker compose configuration

In `docker/compose/devnet.yaml` (lines 290-308), the secondary node service:

- Only maps the P2P port (`30500:30303`) -- no RPC, WebSocket, or metrics ports exposed
- Uses `HEALTHCHECK_MODE=p2p` which only checks `nc -z localhost 30303` -- always passes
- Does not receive `genesis.json` or any DKG output
- Does not set `VALIDATOR_INDEX` or `VALIDATOR_COUNT` since these are consensus-only parameters

### Observed runtime behavior

- Zero CPU usage, ~7 MB memory
- No log output after initial "secondary peer joined network"
- No warnings or errors when validators are stopped
- The `/data/` directory contains only `setup.json`, `validator.key`, and `.ready` -- no database files

## Expected behavior

A secondary node should follow the validator chain in real time and serve read-only RPC queries. It should:

1. Receive finalized blocks from validators via the marshal actor stack (not raw broadcast)
2. Verify finalization certificates using the validators' BLS threshold public key
3. Execute each block against its local QMDB state using the same `RevmExecutor` as validators
4. Index blocks, transactions, receipts, and logs for RPC queries
5. Serve the standard Ethereum JSON-RPC interface (minus `eth_sendRawTransaction`, or with transaction forwarding to a validator)
6. Export Prometheus metrics (block height, sync lag, RPC latency)
7. Detect and report when it falls behind the validator chain

## Architectural context

### How validators process finalized blocks (the reference pipeline)

The validator node (`ProductionRunner` in `crates/node/runner/src/runner.rs`) has a multi-layered stack for processing finalized blocks. Understanding this stack is critical because the secondary node must reuse most of it:

```
simplex consensus engine
    |
    v  (Activity reports: notarizations, finalizations, nullifications)
Reporters tuple: (SeedReporter, (MarshalMailbox, Option<NodeStateReporter>))
    |
    v  (marshal_mailbox receives finalization certificates)
Marshal Actor (ActorInitializer::init_with_strategy)
    |
    |-- Archives finalization certificates to finalizations_by_height
    |-- Archives finalized blocks to finalized_blocks
    |-- Verifies certificates using ConstantSchemeProvider (ThresholdScheme)
    |-- Delivers Update::Block(block, ack) to FinalizedReporter
    |-- Sends blocks to broadcast engine buffer for P2P dissemination
    |-- Uses PeerInitializer resolver for backfill/catch-up
    |
    v
FinalizedReporter::report(Update<Block>)
    |-- handle_finalized_update()
    |   |-- finalize_block(): fetch parent snapshot, execute via RevmExecutor,
    |   |   verify state root, persist to QMDB
    |   |-- index_finalized_block(): populate BlockIndex for RPC
    |   |-- prune_mempool(): remove included txs
    |   |-- ack.acknowledge(): tell marshal to advance delivery floor
    |
    v
BlockIndex + QMDB state
    |
    v
RpcServer (IndexedStateProvider backed by BlockIndex + QMDB)
```

### The key insight: marshal actor handles block RECEPTION

**CRITICAL:** The broadcast engine + marshal actor stack handles both sending AND receiving finalized blocks. On validators, the flow is:

1. **Broadcast engine** (`BroadcastInitializer::init`) creates an `(Engine, Mailbox)` pair. The `Mailbox` is the send-side (used by the simplex engine to broadcast blocks). The `Engine::start(transport.marshal.blocks)` connects to the P2P blocks channel to both send and receive.

2. **Marshal actor** (`ActorInitializer::init_with_strategy`) receives blocks from the broadcast engine buffer, verifies their finalization certificates using the `ConstantSchemeProvider`, archives them, and delivers `Update::Block` to the `FinalizedReporter`.

3. **PeerInitializer resolver** handles backfill/catch-up requests over the `transport.marshal.backfill` channel.

A secondary node needs all three of these components. The secondary does not need the simplex consensus engine, the `SeedReporter`, or the `NodeStateReporter` -- but it absolutely needs the marshal actor to receive, verify, and deliver finalized blocks.

The old pseudocode in this issue incorrectly showed `broadcast_mailbox.recv().await` as the block source. This is wrong -- `broadcast_mailbox` is the **send-side** (for broadcasting blocks to peers). The **receive-side** is handled internally by the marshal actor, which delivers blocks to the `FinalizedReporter` via the `Update::Block` callback.

### Certificate verification key distribution problem

The marshal actor requires a `Provider<Scheme = ThresholdScheme>` to verify finalization certificates. On validators, this is `ConstantSchemeProvider(Arc<ThresholdScheme>)`, which wraps the BLS12-381 threshold scheme loaded from DKG output.

The `ThresholdScheme` is constructed via `bls12381_threshold::Scheme::signer()`, which requires:
- `participants_set`: Ed25519 public keys of all validators (available from `peers.json`)
- `group_poly`: The BLS public polynomial from DKG (available from `output.json`)
- `share`: The node's secret share (from `share.key`)

**Problem:** A secondary node does NOT participate in DKG and does NOT hold a secret share. It needs a **verifier-only** scheme that can verify certificates without signing.

**Solution options:**

1. **Export public-only DKG output:** During DKG or setup, produce a `dkg-public.json` file containing `group_public_key`, `public_polynomial`, `participant_keys`, and `threshold` -- everything except `share_secret`. The secondary loads this and constructs a verify-only scheme. This requires either a `Scheme::verifier()` constructor in commonware or a wrapper that panics on sign attempts.

2. **Share DKG output.json with secondary:** The `output.json` file already contains all the public DKG parameters (group key, polynomial, participant keys). Copy it to the secondary's shared config. The secondary only needs public polynomial for verification. The `share.key` file is NOT needed for verification-only mode.

3. **Skip certificate verification (devnet shortcut):** For early development, the secondary could trust all blocks from known validators without certificate verification. This is insecure but unblocks the basic block-following functionality. The marshal actor would need a no-op scheme provider.

**Recommended approach:** Option 2 for initial implementation. The `output.json` is already a shared artifact and contains no secrets. The implementation will need a `load_threshold_scheme_verifier()` function that loads `output.json` without requiring `share.key`. If `bls12381_threshold::Scheme` has no verifier-only constructor, a dummy share can be generated (it will never be used for signing). This needs investigation of the commonware API.

### What the secondary should NOT do

- Participate in consensus (no simplex engine, no voting, no certificates)
- Sign threshold signatures (may hold public DKG parameters for verification, but never signs)
- Propose blocks
- Maintain a mempool (no local transaction ordering)

## Proposed implementation

### Phase 1: Block following + state building (minimum viable)

Create a new `SecondaryRunner` (analogous to `ProductionRunner`) that initializes the full marshal actor stack for receiving and processing finalized blocks.

#### CLI changes (`bin/kora/src/cli.rs`)

Extend `SecondaryArgs` with the parameters needed by the follower:

```rust
#[derive(clap::Args, Debug)]
pub(crate) struct SecondaryArgs {
    /// Path to peers.json file containing primary and secondary peer information.
    #[arg(long)]
    pub peers: PathBuf,

    /// Prometheus metrics server bind address.
    #[arg(long, default_value = "0.0.0.0:9002")]
    pub metrics_addr: String,

    /// JSON-RPC server bind address.
    #[arg(long, default_value = "0.0.0.0:8545")]
    pub rpc_addr: String,
}
```

Update `run_secondary()` to:
1. Load the DKG public parameters (from `output.json` copied to shared config)
2. Load genesis configuration
3. Construct a `SecondaryRunner` and call `runner.run()`

```rust
// bin/kora/src/cli.rs -- updated run_secondary()
fn run_secondary(&self, args: &SecondaryArgs) -> eyre::Result<()> {
    use commonware_runtime::Runner;
    use kora_transport::NetworkConfigExt;

    let mut config = self.load_config()?;
    let peers = load_peers(&args.peers)?;
    config.network.bootstrap_peers = format_bootstrappers(&peers.bootstrappers);

    let identity_key = config.validator_key()?;
    let my_pk = commonware_cryptography::Signer::public_key(&identity_key);
    if !peers.secondary_participants.contains(&my_pk) {
        return Err(eyre::eyre!(
            "secondary identity is not listed in peers.json secondary_participants"
        ));
    }

    // Load genesis
    let genesis_path = config.data_dir.join("genesis.json");
    let bootstrap = BootstrapConfig::load(&genesis_path)
        .map_err(|e| eyre::eyre!("Failed to load genesis: {}", e))?;

    // Load DKG public parameters for certificate verification (no share needed)
    let scheme = load_threshold_scheme_verifier(&config.data_dir)
        .map_err(|e| eyre::eyre!("Failed to load verification scheme: {}", e))?;

    let rpc_addr: std::net::SocketAddr = args.rpc_addr.parse()
        .map_err(|e| eyre::eyre!("invalid --rpc-addr: {}", e))?;
    let metrics_addr: std::net::SocketAddr = args.metrics_addr.parse()
        .map_err(|e| eyre::eyre!("invalid --metrics-addr: {}", e))?;

    let node_state = NodeState::new(config.chain_id, 0);

    tracing::info!(
        chain_id = config.chain_id,
        bootstrap_peers = config.network.bootstrap_peers.len(),
        secondary_peers = peers.secondary_participants.len(),
        "Starting secondary peer"
    );

    let runner = SecondaryRunner {
        scheme,
        chain_id: config.chain_id,
        bootstrap,
        rpc_config: Some((node_state, rpc_addr)),
        metrics_addr: Some(metrics_addr),
        secondary_peers: peers.secondary_participants,
    };

    let runtime_dir = runtime_storage_directory(&config.data_dir);
    let executor = commonware_runtime::tokio::Runner::new(
        commonware_runtime::tokio::Config::default()
            .with_storage_directory(runtime_dir),
    );
    executor.start(|context| async move {
        let transport = config
            .network
            .build_local_transport(identity_key, context.clone())
            .map_err(|e| eyre::eyre!("failed to build transport: {}", e))?;

        // Register all peers with the discovery oracle
        transport.oracle.track(
            0,
            TrackedPeers::new(
                Set::from_iter_dedup(peers.participants),
                Set::from_iter_dedup(runner.secondary_peers.clone()),
            ),
        ).await;

        runner.run(context, config).await
            .map_err(|e| eyre::eyre!("secondary runner failed: {}", e))?;

        tokio::signal::ctrl_c().await.ok();
        Ok::<(), eyre::Error>(())
    })
}
```

#### Verification-only scheme loader (`crates/node/runner/src/scheme.rs`)

```rust
/// Load a threshold scheme configured for verification only (no signing).
///
/// This loads `output.json` for the public polynomial and participant keys
/// but does NOT require `share.key`. Used by secondary nodes that need to
/// verify finalization certificates but never produce threshold signatures.
pub fn load_threshold_scheme_verifier(data_dir: &Path) -> anyhow::Result<ThresholdScheme> {
    // Load only output.json (NOT share.key)
    let output_path = data_dir.join("output.json");
    let output_str = std::fs::read_to_string(&output_path)
        .map_err(|e| anyhow::anyhow!("failed to read output.json: {e}"))?;
    let output: serde_json::Value = serde_json::from_str(&output_str)?;

    let participants: Vec<ed25519::PublicKey> = /* decode from output.participant_keys */;
    let n = participants.len();
    let n_cfg = NonZeroU32::new(n as u32)
        .ok_or_else(|| anyhow::anyhow!("participants cannot be empty"))?;
    let participants_set = Set::from_iter_dedup(participants);

    let public_polynomial_hex = output["public_polynomial"].as_str()
        .ok_or_else(|| anyhow::anyhow!("missing public_polynomial"))?;
    let poly_bytes = hex::decode(public_polynomial_hex)?;
    let group_poly = Sharing::<MinSig>::read_cfg(
        &mut poly_bytes.as_slice(),
        &(n_cfg, ModeVersion::v0()),
    ).map_err(|e| anyhow::anyhow!("failed to decode public polynomial: {:?}", e))?;

    // NOTE: This requires investigation of the commonware API.
    // If Scheme::signer() is the only constructor, we may need to:
    // (a) Add a Scheme::verifier() constructor upstream, or
    // (b) Create a dummy share that won't be used for signing, or
    // (c) Use a different verification path
    todo!("construct verify-only ThresholdScheme from public polynomial")
}
```

#### Core secondary runner (`crates/node/runner/src/secondary.rs`)

The secondary runner reuses the same marshal actor stack as `ProductionRunner`, minus the simplex consensus engine. The key architectural difference: on a validator, blocks flow from simplex -> marshal mailbox -> marshal actor -> FinalizedReporter. On a secondary, blocks flow from P2P broadcast channel -> marshal actor -> FinalizedReporter. The marshal actor handles both paths.

```rust
// crates/node/runner/src/secondary.rs (new file)

use std::sync::Arc;

use commonware_consensus::marshal::{core::Mailbox, standard::Standard};
use commonware_runtime::{
    Clock, Handle as RuntimeHandle, Metrics, Spawner,
    buffer::paged::CacheRef, tokio as cw_tokio,
};
use commonware_utils::{NZUsize, acknowledgement::Exact, ordered::Set};
use kora_domain::{Block, BootstrapConfig};
use kora_executor::RevmExecutor;
use kora_indexer::BlockIndex;
use kora_ledger::{LedgerService, LedgerView};
use kora_marshal::{ArchiveInitializer, BroadcastInitializer, PeerInitializer};
use kora_reporters::FinalizedReporter;
use kora_transport::NetworkTransport;
use tracing::{error, info, warn};

use crate::{RunnerError, scheme::ThresholdScheme};
use super::runner::{
    ConstantSchemeProvider, NoOpBlocker, RevmContextProvider,
    block_codec_cfg, default_page_cache, seed_genesis_block_index,
    spawn_ledger_observers,
};

type Peer = commonware_cryptography::ed25519::PublicKey;

pub struct SecondaryRunner {
    pub scheme: ThresholdScheme,
    pub chain_id: u64,
    pub bootstrap: BootstrapConfig,
    pub rpc_config: Option<(kora_rpc::NodeState, std::net::SocketAddr)>,
    pub metrics_addr: Option<std::net::SocketAddr>,
    pub secondary_peers: Vec<Peer>,
}

impl SecondaryRunner {
    pub async fn run(
        &self,
        context: cw_tokio::Context,
        config: kora_config::NodeConfig,
    ) -> Result<(), RunnerError> {
        let gas_limit = config.execution.gas_limit;
        let block_cfg = block_codec_cfg(&config.consensus.block_codec);

        info!(chain_id = self.chain_id, "Starting secondary follower node");

        // --- 1. Initialize QMDB state from genesis ---
        let state = LedgerView::init_with_genesis_options(
            context.with_label("state"),
            format!("kora-secondary-qmdb"),
            self.bootstrap.genesis_alloc.clone(),
            true, // apply genesis on first start
            self.bootstrap.genesis_timestamp,
        )
        .await
        .map_err(|e| RunnerError(e.to_string()))?;

        let ledger = LedgerService::new(state.clone());
        let block_index = Arc::new(BlockIndex::new());
        seed_genesis_block_index(&block_index, &ledger.genesis_block(), gas_limit);
        spawn_ledger_observers(ledger.clone(), context.clone(), config.data_dir.clone());

        let context_provider = RevmContextProvider {
            gas_limit,
            block_index: block_index.clone(),
        };

        // --- 2. Initialize certificate verification ---
        let scheme_provider = ConstantSchemeProvider::from(self.scheme.clone());
        let page_cache = default_page_cache(&context);
        let strategy = context
            .create_strategy(NZUsize!(2))
            .map_err(|e| RunnerError(format!("failed to create strategy: {e}")))?;

        // --- 3. Initialize archive storage for finalized blocks ---
        let partition_prefix = "kora-secondary";

        // Certificate type config needed for archive
        <ThresholdScheme as commonware_cryptography::certificate::Scheme>
            ::certificate_codec_config_unbounded();

        let finalizations_by_height = ArchiveInitializer::init::<_, ConsensusDigest, CertArchive>(
            context.with_label("finalizations_by_height"),
            format!("{partition_prefix}-finalizations-by-height"),
            (),
        )
        .await
        .map_err(|e| RunnerError(format!("init finalizations archive: {e}")))?;

        let finalized_blocks = ArchiveInitializer::init::<_, ConsensusDigest, Block>(
            context.with_label("finalized_blocks"),
            format!("{partition_prefix}-finalized-blocks"),
            block_cfg,
        )
        .await
        .map_err(|e| RunnerError(format!("init blocks archive: {e}")))?;

        // Recover any previously finalized state from archive (restart case)
        recover_finalized_state(
            &ledger, &block_index, &finalized_blocks,
            &finalizations_by_height, &context_provider, &config.data_dir,
        )
        .await
        .map_err(|e| RunnerError(format!("recover finalized state: {e}")))?;

        // --- 4. Initialize marshal actor stack (block reception) ---
        //
        // This is the same stack validators use. The marshal actor:
        // - Receives blocks from the broadcast engine buffer
        // - Verifies finalization certificates using scheme_provider
        // - Archives blocks and certificates
        // - Delivers Update::Block to our FinalizedReporter
        let my_pk = /* loaded from config.validator_key() */;

        let resolver = PeerInitializer::init::<_, _, _, Block, _, _, _>(
            &context.with_label("resolver"),
            my_pk.clone(),
            transport.oracle.clone(),
            NoOpBlocker::<Peer>::new(),
            transport.marshal.backfill,
        );

        let (broadcast_engine, buffer) = BroadcastInitializer::init::<_, Peer, Block, _>(
            context.with_label("broadcast"),
            my_pk.clone(),
            transport.oracle.clone(),
            block_cfg,
        );
        let broadcast_handle = broadcast_engine.start(transport.marshal.blocks);

        let (actor, marshal_mailbox, _last_height) =
            kora_marshal::ActorInitializer::init_with_strategy::<_, Block, _, _, _, Exact, _>(
                context.clone(),
                finalizations_by_height,
                finalized_blocks,
                scheme_provider,
                page_cache,
                block_cfg,
                strategy,
            )
            .await;

        // --- 5. Build FinalizedReporter (reuses validator's finalization pipeline) ---
        let finalized_executor = RevmExecutor::new(self.chain_id);
        let finalized_reporter = FinalizedReporter::new(
            ledger.clone(),
            context.clone(),
            finalized_executor,
            context_provider,
        )
        .with_block_index(block_index.clone());

        // Start the marshal actor -- it will call finalized_reporter.report()
        // for each finalized block delivered via the broadcast channel
        let marshal_handle = actor.start(finalized_reporter, buffer, resolver);

        // --- 6. Start RPC server (read-only, no tx_submit) ---
        if let Some((node_state, addr)) = &self.rpc_config {
            let qmdb_state = state.qmdb_state().await;
            let rpc_executor = Arc::new(RevmExecutor::new(self.chain_id));
            let indexed_provider = kora_rpc::IndexedStateProvider::new(
                block_index.clone(), qmdb_state, rpc_executor,
            );
            // No tx_submit callback -- eth_sendRawTransaction returns error
            let rpc = kora_rpc::RpcServer::with_state_provider(
                node_state.clone(), *addr, self.chain_id, indexed_provider,
            );
            drop(rpc.start());
            info!(addr = %addr, "Secondary RPC server started (read-only)");
        }

        // --- 7. Start metrics server ---
        if let Some(metrics_addr) = self.metrics_addr {
            let metrics_context = context.clone();
            context.with_label("metrics").shared(true).spawn(move |_| async move {
                let app = axum::Router::new().route(
                    "/metrics",
                    axum::routing::get(move || {
                        let body = metrics_context.encode();
                        async move {
                            (
                                axum::http::StatusCode::OK,
                                [(
                                    axum::http::header::CONTENT_TYPE,
                                    "application/openmetrics-text; version=1.0.0; charset=utf-8",
                                )],
                                body,
                            )
                        }
                    }),
                );
                let listener = match tokio::net::TcpListener::bind(metrics_addr).await {
                    Ok(l) => l,
                    Err(e) => {
                        error!(addr = %metrics_addr, error = %e, "Failed to bind metrics server");
                        return;
                    }
                };
                info!(addr = %metrics_addr, "Starting secondary metrics server");
                if let Err(e) = axum::serve(listener, app).await {
                    error!(error = %e, "Metrics server error");
                }
            });
        }

        // --- 8. Monitor critical tasks ---
        spawn_task_watchdog(&context, "marshal_actor", marshal_handle);
        spawn_task_watchdog(&context, "broadcast_engine", broadcast_handle);

        info!("Secondary node started successfully");
        Ok(())
    }
}
```

#### How this differs from the old (incorrect) pseudocode

The previous version of this issue described the secondary as directly consuming from a "broadcast_mailbox":

```rust
// WRONG -- this is the SEND side, not the receive side
loop {
    let block = broadcast_mailbox.recv().await;
    finalize_and_index(..., block).await;
}
```

This is incorrect for two reasons:

1. **`BroadcastInitializer::init()` returns `(Engine, Mailbox)` where `Mailbox` is the SEND side.** The `Mailbox` is used to submit blocks for broadcast to peers. There is no `recv()` on the broadcast mailbox.

2. **Block reception is handled by the marshal actor internally.** The broadcast `Engine::start(transport.marshal.blocks)` connects to the P2P blocks channel. Received blocks flow through the engine's buffer into the marshal actor, which verifies certificates, archives them, and delivers them to the `FinalizedReporter` via the `Reporter::report()` trait method.

The correct architecture is to instantiate the full marshal actor stack (exactly as `ProductionRunner::run()` does at lines 695-724 of `runner.rs`) and provide a `FinalizedReporter` as the block delivery callback. The secondary omits only the simplex consensus engine and the consensus-specific reporters (SeedReporter, NodeStateReporter).

#### File changes for Phase 1

| File | Change |
|---|---|
| `crates/node/runner/src/secondary.rs` | **New file**: `SecondaryRunner` using marshal actor stack |
| `crates/node/runner/src/lib.rs` | Add `mod secondary; pub use secondary::SecondaryRunner;` |
| `crates/node/runner/src/scheme.rs` | Add `load_threshold_scheme_verifier()` for verify-only mode |
| `crates/node/runner/src/runner.rs` | Make `ConstantSchemeProvider`, `NoOpBlocker`, `RevmContextProvider`, `block_codec_cfg`, `default_page_cache`, `seed_genesis_block_index`, `spawn_ledger_observers`, `spawn_task_watchdog`, `recover_finalized_state` pub(crate) so secondary.rs can reuse them |
| `bin/kora/src/cli.rs` | Extend `SecondaryArgs`, rewrite `run_secondary()` to use `SecondaryRunner` |
| `docker/compose/devnet.yaml` | Add RPC + metrics port mappings to `secondary-node0` service |
| `docker/scripts/entrypoint.sh` | Copy `genesis.json` and `output.json` to secondary; pass `--rpc-addr` and `--metrics-addr` |
| `docker/scripts/healthcheck.sh` | Add `secondary` healthcheck mode that checks RPC |

### Phase 2: RPC server + full indexing

Wire up the full RPC server with:

- `eth_blockNumber`, `eth_getBlockByNumber`, `eth_getBlockByHash` -- from block index
- `eth_getBalance`, `eth_getTransactionCount`, `eth_getCode`, `eth_getStorageAt` -- from QMDB state
- `eth_call`, `eth_estimateGas` -- from `RevmExecutor` against QMDB state
- `eth_getTransactionByHash`, `eth_getTransactionReceipt` -- from block index
- `eth_getLogs` -- from block index
- `eth_chainId`, `net_version`, `web3_clientVersion` -- static values
- `eth_sendRawTransaction` -- reject with "secondary node does not accept transactions"

This is largely plug-and-play: `IndexedStateProvider` already implements the full `StateProvider` trait. The secondary just needs to construct one with the block index and QMDB state, exactly as `ProductionRunner::run()` does at lines 571-574 of `crates/node/runner/src/runner.rs`:

```rust
// From ProductionRunner::run() -- reusable as-is
let qmdb_state = state.qmdb_state().await;
let rpc_executor = Arc::new(RevmExecutor::new(self.chain_id));
let indexed_provider =
    kora_rpc::IndexedStateProvider::new(block_index.clone(), qmdb_state, rpc_executor);
```

Add Prometheus metrics export (same as validator -- an axum `/metrics` endpoint using `context.encode()`).

### Phase 3: Catch-up sync

When a secondary node starts for the first time (or restarts after being offline), it needs to catch up to the validator chain head. The marshal actor already supports this via the PeerInitializer resolver.

**Built-in catch-up via marshal actor:** The marshal actor's resolver (initialized via `PeerInitializer::init()`) automatically requests missing blocks from peers via the backfill channel (`CHANNEL_BACKFILL`, channel 4). This is the same mechanism validators use to recover after restart (see `recover_finalized_state()` in `runner.rs`). The secondary gets this for free by using the full marshal actor stack.

**Limitation:** The resolver may struggle with large gaps due to the `NoOpBlocker` workaround for transient verification failures. If a secondary starts for the first time against a chain with thousands of blocks, re-executing every block is expensive. A state snapshot transfer mechanism would be a future optimization.

### Phase 4: Production hardening

- Sync lag alerting: export a `kora_secondary_sync_lag` metric showing how many blocks behind the chain head the secondary is
- Automatic reconnection if all validator peers disconnect
- State root verification on every block (already part of the `finalize_block()` logic)
- Graceful degradation: if the secondary falls too far behind, switch RPC responses to return a "syncing" status via `eth_syncing`
- Multiple secondary nodes behind a load balancer for RPC traffic

## Docker compose changes

Update the `secondary-node0` service in `docker/compose/devnet.yaml`:

```yaml
secondary-node0:
  <<: *validator-common
  hostname: secondary0
  depends_on:
    validator-node0:
      condition: service_healthy
  entrypoint: ["/scripts/entrypoint.sh", "secondary"]
  volumes:
    - shared_config:/shared:ro
    - data_secondary0:/data
  environment:
    - RUST_LOG=${RUST_LOG:-info}
    - CHAIN_ID=${CHAIN_ID:-1337}
    - KORA_RUNTIME_DIR=${KORA_RUNTIME_DIR:-/runtime}
    - IS_BOOTSTRAP=false
    - BOOTSTRAP_PEERS=node0:30303
    - HEALTHCHECK_MODE=ready    # Changed from p2p to ready (RPC check)
  ports:
    - "30500:30303"   # P2P
    - "8549:8545"     # JSON-RPC (new)
    - "9004:9002"     # Prometheus metrics (new)
```

The entrypoint script (`docker/scripts/entrypoint.sh`) secondary case needs to:
1. Copy `genesis.json` from `/shared/` to `/data/` (same as validators do)
2. Copy `output.json` from `/shared/` to `/data/` (for certificate verification)
3. Pass `--rpc-addr 0.0.0.0:8545` and `--metrics-addr 0.0.0.0:9002` to `kora secondary`

```bash
# docker/scripts/entrypoint.sh -- updated secondary case
secondary)
    log "Running secondary peer mode..."

    [[ -f "${SHARED_DIR}/peers.json" ]] || error "peers.json not found"
    [[ -f "${SHARED_DIR}/genesis.json" ]] || error "genesis.json not found"
    [[ -f "${SHARED_DIR}/output.json" ]] || error "output.json not found (needed for cert verification)"
    [[ -f "${DATA_DIR}/validator.key" ]] || error "validator.key not found"

    cp "${SHARED_DIR}/genesis.json" "${DATA_DIR}/" 2>/dev/null || true
    cp "${SHARED_DIR}/output.json" "${DATA_DIR}/" 2>/dev/null || true
    touch "${DATA_DIR}/.ready"

    if [[ "$IS_BOOTSTRAP" != "true" && -n "$BOOTSTRAP_PEERS" ]]; then
        BOOTSTRAP_HOST=$(echo "$BOOTSTRAP_PEERS" | cut -d: -f1)
        BOOTSTRAP_PORT=$(echo "$BOOTSTRAP_PEERS" | cut -d: -f2)

        log "Waiting for bootstrap peer ${BOOTSTRAP_HOST}:${BOOTSTRAP_PORT}..."
        timeout=120
        while ! nc -z "$BOOTSTRAP_HOST" "$BOOTSTRAP_PORT" 2>/dev/null; do
            timeout=$((timeout - 1))
            [[ $timeout -le 0 ]] && error "Timeout waiting for bootstrap peer"
            sleep 1
        done
    fi

    exec /usr/local/bin/kora secondary \
        --data-dir "$DATA_DIR" \
        --peers "${SHARED_DIR}/peers.json" \
        --chain-id "$CHAIN_ID" \
        --rpc-addr "0.0.0.0:8545" \
        --metrics-addr "0.0.0.0:9002" \
        "$@"
    ;;
```

## Open questions

1. **Verifier-only ThresholdScheme:** Does `bls12381_threshold::Scheme` in commonware support a verifier-only constructor (no secret share)? If not, what is the cleanest workaround? Options: dummy share, upstream PR, or a wrapper type.

2. **output.json sharing:** Is `output.json` safe to share with secondary nodes? It contains the public polynomial and group public key but NOT secret shares. This should be safe (these are public parameters by design), but needs confirmation.

3. **SeedReporter dependency:** The `FinalizedReporter` uses `LedgerService::set_seed()` for VRF seeds, which on validators is populated by `SeedReporter` from consensus activity. Since the secondary has no consensus engine, seeds won't be populated via that path. The recover path in `recover_finalized_state()` does populate seeds from archived finalizations. Need to verify that live block delivery via the marshal actor also provides seed information, or if a secondary-specific seed extraction from finalization certificates is needed.

## Acceptance criteria

### Phase 1 (minimum viable)
- [ ] `kora secondary` initializes the full marshal actor stack (broadcast engine, resolver, actor)
- [ ] Finalization certificates are verified using the validators' BLS threshold public key
- [ ] `FinalizedReporter` receives blocks and executes them against QMDB
- [ ] State root is verified against block header on every finalized block
- [ ] State is persisted to QMDB; the secondary's `/data/` directory grows with database files
- [ ] Secondary survives validator restarts and catches up on missed blocks via resolver

### Phase 2 (RPC + metrics)
- [ ] `eth_blockNumber` returns the latest finalized block height
- [ ] `eth_getBalance` returns correct balances matching validators
- [ ] `eth_call` and `eth_estimateGas` work against the secondary's state
- [ ] `eth_getTransactionReceipt` and `eth_getLogs` return indexed data
- [ ] Prometheus `/metrics` endpoint exports block height and sync lag
- [ ] Docker healthcheck uses `HEALTHCHECK_MODE=ready` (RPC probe)

### Phase 3 (catch-up sync)
- [ ] A freshly started secondary catches up to the chain head via resolver backfill
- [ ] Re-syncing after being offline for N blocks works correctly
- [ ] State root matches validators at every height during catch-up

## Estimated effort

- Phase 1: 3-5 days (marshal actor integration, certificate verification scheme, block following)
- Phase 2: 2-3 days (RPC server wiring, mostly reusing existing code)
- Phase 3: 1-2 days (resolver backfill is built-in; testing and edge cases)
- Phase 4: 2-3 days (metrics, alerting, production hardening)

Total: ~2-3 weeks for full implementation.

## Related files

- `bin/kora/src/cli.rs` -- CLI entry point with `run_secondary()` stub (lines 194-246)
- `crates/node/runner/src/runner.rs` -- `ProductionRunner` (validator reference implementation, especially lines 695-724 for marshal actor setup)
- `crates/node/runner/src/scheme.rs` -- `ThresholdScheme` and `load_threshold_scheme()` (needs verifier-only variant)
- `crates/node/runner/src/lib.rs` -- Runner module exports
- `crates/node/reporters/src/lib.rs` -- `FinalizedReporter`, `handle_finalized_update()`, `finalize_block()` (the block execution pipeline to reuse)
- `crates/node/ledger/src/lib.rs` -- `LedgerView` / `LedgerService` (state management)
- `crates/node/executor/src/revm.rs` -- `RevmExecutor` (EVM execution)
- `crates/node/executor/src/traits.rs` -- `BlockExecutor` trait
- `crates/node/rpc/src/server.rs` -- `RpcServer` (JSON-RPC server)
- `crates/node/rpc/src/indexed_provider.rs` -- `IndexedStateProvider` (RPC state queries)
- `crates/node/rpc/src/state_provider.rs` -- `StateProvider` trait
- `crates/node/dkg/src/output.rs` -- `DkgOutput` struct (public parameters for verification)
- `crates/network/transport/src/transport.rs` -- `NetworkTransport` (P2P transport bundle)
- `crates/network/transport/src/channels.rs` -- P2P channel definitions (CHANNEL_BLOCKS = 3, CHANNEL_BACKFILL = 4)
- `crates/network/marshal/src/actor.rs` -- `ActorInitializer` (marshal actor init with certificate verification)
- `crates/network/marshal/src/broadcast.rs` -- `BroadcastInitializer` (broadcast engine -- Engine is receive+send, Mailbox is send-only)
- `crates/network/marshal/src/peers.rs` -- `PeerInitializer` (resolver for backfill catch-up)
- `docker/compose/devnet.yaml` -- Docker compose for devnet (secondary-node0 at line 290)
- `docker/scripts/entrypoint.sh` -- Container entrypoint script (secondary case at line 126)
- `docker/scripts/healthcheck.sh` -- Container healthcheck script

## Labels

`enhancement`, `node`, `P2P`, `RPC`
