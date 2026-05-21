//! daeji-agent-indexer — off-chain event indexer for agent coordination.
//!
//! Subscribes to `AgentRegistry.AgentRegistered` and `MultiAgentMarket.JobAwarded`
//! events on a JSON-RPC WebSocket endpoint, materializes views in memory, and
//! exposes a small HTTP API the agent runtime + UI can query for discovery.
//!
//! Per canonical-plan §D3: the chain remains source of truth; the indexer is a
//! materialized cache. No on-chain "indexer" is added to avoid duplicate write
//! costs.
//!
//! Endpoints (MVP):
//! - `GET /health`           — `{"status":"ok","agents":N,"jobs":M}`
//! - `GET /agents`           — list of all known agents
//! - `GET /agents/:address`  — single agent by 0x-hex EVM address
//! - `GET /jobs`             — list of all known jobs
//! - `GET /jobs/:id`         — single job by chain id
//! - `GET /room/:job_id`     — channel binding (slot index + room id) for a job
//!
//! Filters on `/agents` and `/jobs` are deliberately omitted from the MVP —
//! callers fetch the full lists and filter client-side. Server-side filtering
//! lands in a follow-up PR once the schema-aware routing decision (Q-Open-5) is
//! finalized.

#![allow(missing_docs)]

use std::{
    collections::HashMap,
    net::SocketAddr,
    str::FromStr,
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};

use alloy::{
    primitives::{keccak256, Address, FixedBytes, Log as AlloyLog, U256},
    providers::{Provider, ProviderBuilder, WsConnect},
    rpc::types::eth::{Filter, Log as AlloyRpcLog},
    sol,
    sol_types::SolEvent,
};
use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    routing::{get, post},
    Json, Router,
};
use clap::Parser;
use eyre::{Context, Result};
use futures::StreamExt;
use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;
use tower_http::{cors::CorsLayer, trace::TraceLayer};
use tracing::{error, info, warn};

sol! {
    #[derive(Debug)]
    event AgentRegistered(address indexed agent, bytes32 passportHash, string capabilities);

    #[derive(Debug)]
    event JobAwarded(uint256 indexed id, address[] winners, bytes32 roomId);

    /// ISFR v3.0 single-keeper fast-path submission. Mirrors `IISFROracle.sol`
    /// (PR #102 / R1 in contracts-core).
    #[derive(Debug)]
    event RateSubmitted(
        uint32 indexed epochId,
        address indexed keeper,
        int256 compositeBps,
        uint16 confidenceBps,
        uint64 timestamp
    );

    /// ISFR v3.0 block-range close (Appendix A multi-voter). Mirrors `IISFROracle.sol`.
    #[derive(Debug)]
    event RangeClosed(
        bytes32 indexed rangeId,
        uint64 rangeStart,
        uint64 rangeEnd,
        int256 compositeBps,
        uint16 voterCount
    );

    /// WorkerRegistry events for tier filtering. Mirrors `WorkerRegistry.sol`.
    #[derive(Debug)]
    event WorkerRegistered(address indexed worker, uint256 bond);

    #[derive(Debug)]
    event ReputationUpdated(address indexed worker, uint256 oldRep, uint256 newRep, bool outcome);

    #[derive(Debug)]
    event WorkerSlashed(address indexed worker, uint8 reasonCode, uint256 amount, uint256 newBond);
}

#[derive(Debug, Parser)]
#[command(
    name = "daeji-indexer",
    version,
    about = "off-chain agent + job indexer for the daeji agent-coordination layer"
)]
struct Cli {
    /// JSON-RPC WebSocket endpoint (e.g., ws://127.0.0.1:8545).
    #[arg(long, env = "DAEJI_INDEXER_RPC_WS")]
    rpc_ws: String,

    /// AgentRegistry contract address (0x-hex). When set, indexer subscribes to
    /// AgentRegistered.
    #[arg(long, env = "DAEJI_INDEXER_AGENT_REGISTRY")]
    agent_registry: Option<String>,

    /// MultiAgentMarket contract address (0x-hex). When set, indexer subscribes
    /// to JobAwarded.
    #[arg(long, env = "DAEJI_INDEXER_MARKET")]
    market: Option<String>,

    /// ISFROracle contract addresses (0x-hex, comma-separated for multiple markets).
    /// Each gets a separate `RateSubmitted` + `RangeClosed` subscription. The
    /// per-market deployments are surfaced via `GET /isfr/markets`.
    #[arg(long, env = "DAEJI_INDEXER_ISFR_ORACLES", value_delimiter = ',')]
    isfr_oracles: Vec<String>,

    /// WorkerRegistry contract address (0x-hex). When set, indexer subscribes to
    /// `WorkerRegistered` / `ReputationUpdated` / `WorkerSlashed` and exposes
    /// per-worker tier + reputation_bps + bond. Enables the `?min_tier=` filter
    /// on `/agents` for reputation-gated job discovery (canonical-plan §0 row
    /// "Reputation-gated").
    #[arg(long, env = "DAEJI_INDEXER_WORKER_REGISTRY")]
    worker_registry: Option<String>,

    /// Block to start scanning from. Defaults to the chain head at startup.
    #[arg(long, env = "DAEJI_INDEXER_FROM_BLOCK")]
    from_block: Option<u64>,

    /// Bind address for the HTTP server. Defaults to `127.0.0.1:8787`.
    #[arg(long, env = "DAEJI_INDEXER_BIND", default_value = "127.0.0.1:8787")]
    bind: SocketAddr,
}

/// Materialized agent record. Capabilities is the raw on-chain string; the
/// caller is expected to parse `endpoint=URL` and other tags. The indexer does
/// NOT fetch the off-chain agent card here — the chat-side `chain.rs` watcher
/// already does that and writes a verified registry file. v2 will consolidate.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct Agent {
    address: String,
    passport_hash: String,
    capabilities: String,
    block: u64,
    seen_at_unix: u64,
}

/// Materialized job record. Resolves the deterministic `roomId` against the
/// off-chain derivation so the API caller doesn't need to recompute it.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct Job {
    id: u64,
    winners: Vec<String>,
    room_id: String,
    block: u64,
    seen_at_unix: u64,
}

/// Channel binding response. `slot_index` is `keccak("DAEJI_ROOM_V1" || id)[..4] mod 64`;
/// `channel_id` is the slot's commonware channel id (for direct verification).
#[derive(Debug, Clone, Serialize)]
struct RoomBinding {
    job_id: u64,
    room_id: String,
    slot_index: u32,
    channel_id: u64,
    winners: Vec<String>,
}

/// Latest `RateSubmitted` per ISFR oracle deployment. The keeper, the rate, and
/// the epoch are recorded; consumers query `GET /isfr/rates?market=<addr>` for
/// the most recent. Range-close summaries land in `IsfrRangeClose`.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct IsfrRate {
    market: String,
    epoch_id: u32,
    keeper: String,
    composite_bps: String, // int256 — kept as a decimal string to preserve sign + range
    confidence_bps: u16,
    timestamp: u64,
    block: u64,
    seen_at_unix: u64,
}

/// Worker reputation/bond/tier snapshot. Mirrors `WorkerRegistry.sol`'s view.
/// `tier` is computed locally from `reputation` (matching the on-chain thresholds);
/// decay is NOT applied — the indexer's tier reflects last-known reputation, not
/// the lazily-decayed view. Consumers wanting strict accuracy call the contract
/// directly. Decay-aware indexer tier is a follow-up.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct Worker {
    address: String,
    bond: String,           // uint256 as decimal string (avoid JSON precision loss)
    reputation: u32,        // 0..1_000_000 raw scale
    reputation_bps: u16,    // 0..10000 (reputation / 100)
    tier: WorkerTier,
    block: u64,
    seen_at_unix: u64,
}

/// Tier mirrors `WorkerRegistry.Tier` enum. Order is significant — used for
/// `min_tier` filtering (each tier must be `>=` the requested level).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
enum WorkerTier {
    Unregistered,
    Probation,
    Standard,
    Trusted,
    Elite,
}

impl WorkerTier {
    /// Compute tier from raw reputation (0..1_000_000). Mirrors
    /// `WorkerRegistry.tier()` view. Does NOT factor in `bond < MIN_BOND`
    /// demotion — caller checks bond separately if needed.
    fn from_reputation(rep: u32) -> Self {
        if rep < 350_000 {
            Self::Probation
        } else if rep < 550_000 {
            Self::Standard
        } else if rep < 800_000 {
            Self::Trusted
        } else {
            Self::Elite
        }
    }
}

impl std::str::FromStr for WorkerTier {
    type Err = String;
    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        match s.to_ascii_lowercase().as_str() {
            "unregistered" => Ok(Self::Unregistered),
            "probation" => Ok(Self::Probation),
            "standard" => Ok(Self::Standard),
            "trusted" => Ok(Self::Trusted),
            "elite" => Ok(Self::Elite),
            _ => Err(format!("unknown tier: {s}")),
        }
    }
}

/// Latest `RangeClosed` per ISFR oracle deployment + rangeId. Consumers query
/// `GET /isfr/ranges?market=<addr>` to see closed ranges.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct IsfrRangeClose {
    market: String,
    range_id: String,
    range_start: u64,
    range_end: u64,
    composite_bps: String, // int256 decimal string
    voter_count: u16,
    block: u64,
    seen_at_unix: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct TradingFill {
    fill_id: String,
    trader: String,
    job_id: Option<u64>,
    market: String,
    side: String,
    notional_usd: f64,
    pnl_usd: f64,
    fee_usd: f64,
    timestamp_unix: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct TradingPnLClaim {
    claim_id: String,
    job_id: Option<u64>,
    trader: String,
    gross_earnings: String,
    protocol_fee_bps: u16,
    fills_root: String,
    metadata_hash: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct TraderSnapshot {
    trader_id: String,
    score: f64,
    tier: String,
    pnl_usd: f64,
    fees_usd: f64,
    max_drawdown_bps: u16,
    fill_count: usize,
    fills_root: String,
    last_seen_unix: u64,
    claim: TradingPnLClaim,
}

#[derive(Debug, Deserialize)]
struct TradingIngestRequest {
    fills: Vec<TradingFill>,
    protocol_fee_bps: Option<u16>,
}

#[derive(Default)]
struct Store {
    agents: HashMap<String, Agent>, // 0x-lower address → Agent
    jobs: HashMap<u64, Job>,        // chain id → Job
    /// (market_addr_lower, latest_rate). We only retain the most recent per market.
    isfr_rates: HashMap<String, IsfrRate>,
    /// (market_addr_lower, range_id_lower) → latest close summary. Multiple ranges
    /// can be open per market; we keep them all.
    isfr_ranges: HashMap<(String, String), IsfrRangeClose>,
    /// Per-worker reputation/tier snapshots for `?min_tier=` filtering on /agents.
    workers: HashMap<String, Worker>, // 0x-lower address → Worker
    /// Per-trader Hyperliquid PnL snapshots derived from verified fill batches.
    trader_snapshots: HashMap<String, TraderSnapshot>,
    /// Seen fill ids, used to reject duplicate accounting windows.
    trading_fill_ids: HashMap<String, bool>,
}

type SharedStore = Arc<RwLock<Store>>;

#[tokio::main]
async fn main() -> Result<()> {
    let _ = tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info,daeji_indexer=info".into()),
        )
        .try_init();

    let cli = Cli::parse();
    info!(
        rpc_ws = %cli.rpc_ws,
        agent_registry = ?cli.agent_registry,
        market = ?cli.market,
        isfr_oracle_count = cli.isfr_oracles.len(),
        bind = %cli.bind,
        "daeji-indexer: starting"
    );

    let store: SharedStore = Arc::new(RwLock::new(Store::default()));

    let agent_addr = parse_optional_address("agent_registry", cli.agent_registry.as_deref())?;
    let market_addr = parse_optional_address("market", cli.market.as_deref())?;
    let isfr_addrs = cli
        .isfr_oracles
        .iter()
        .map(|s| {
            Address::from_str(s.trim())
                .with_context(|| format!("invalid ISFR oracle address: {}", s))
        })
        .collect::<Result<Vec<_>>>()?;
    let worker_registry_addr =
        parse_optional_address("worker_registry", cli.worker_registry.as_deref())?;
    if agent_addr.is_none()
        && market_addr.is_none()
        && isfr_addrs.is_empty()
        && worker_registry_addr.is_none()
    {
        warn!("daeji-indexer: no event sources configured (--agent-registry / --market / --isfr-oracles / --worker-registry); HTTP server will only return empty results");
    }

    let watch_store = store.clone();
    let watch_rpc = cli.rpc_ws.clone();
    let watch_from = cli.from_block;
    let watch_isfr = isfr_addrs.clone();
    tokio::spawn(async move {
        if let Err(err) = run_chain_watcher(
            watch_rpc,
            agent_addr,
            market_addr,
            watch_isfr,
            worker_registry_addr,
            watch_from,
            watch_store,
        )
        .await
        {
            error!(?err, "chain watcher exited");
        }
    });

    let app = Router::new()
        .route("/health", get(health))
        .route("/agents", get(list_agents))
        .route("/agents/{address}", get(get_agent))
        .route("/jobs", get(list_jobs))
        .route("/jobs/{id}", get(get_job))
        .route("/room/{job_id}", get(get_room))
        .route("/isfr/markets", get(list_isfr_markets))
        .route("/isfr/rates", get(list_isfr_rates))
        .route("/isfr/rates/{market}", get(get_isfr_rate))
        .route("/isfr/ranges", get(list_isfr_ranges))
        .route("/workers", get(list_workers))
        .route("/workers/{address}", get(get_worker))
        .route("/trading/fills", post(ingest_trading_fills))
        .route("/traders", get(list_trader_snapshots))
        .route("/traders/{trader}", get(get_trader_snapshot))
        .layer(CorsLayer::permissive())
        .layer(TraceLayer::new_for_http())
        .with_state(store.clone());

    let listener = tokio::net::TcpListener::bind(cli.bind)
        .await
        .with_context(|| format!("bind to {}", cli.bind))?;
    info!(bind = %cli.bind, "daeji-indexer: HTTP server listening");

    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await?;
    info!("daeji-indexer: shutting down");
    Ok(())
}

async fn shutdown_signal() {
    let _ = tokio::signal::ctrl_c().await;
    info!("daeji-indexer: ctrl-c received");
}

fn parse_optional_address(field: &str, raw: Option<&str>) -> Result<Option<Address>> {
    raw.map(|s| {
        Address::from_str(s).with_context(|| format!("invalid 0x-hex address for {}: {}", field, s))
    })
    .transpose()
}

async fn run_chain_watcher(
    rpc_ws: String,
    agent_addr: Option<Address>,
    market_addr: Option<Address>,
    isfr_addrs: Vec<Address>,
    worker_registry_addr: Option<Address>,
    from_block_override: Option<u64>,
    store: SharedStore,
) -> Result<()> {
    let provider = ProviderBuilder::new()
        .connect_ws(WsConnect::new(&rpc_ws))
        .await
        .with_context(|| format!("connect to {}", rpc_ws))?;

    let chain_id = provider.get_chain_id().await?;
    let head = provider.get_block_number().await?;
    info!(chain_id, head, "daeji-indexer: chain connected");
    let from_block = from_block_override.unwrap_or(head);

    // Pre-register the ISFR oracle addresses in the store so /isfr/markets
    // surfaces them even before the first event arrives.
    if !isfr_addrs.is_empty() {
        let mut s = store.write().await;
        for addr in &isfr_addrs {
            s.isfr_rates.entry(format!("{:#x}", addr).to_ascii_lowercase()).or_insert(
                IsfrRate {
                    market: format!("{:#x}", addr),
                    epoch_id: 0,
                    keeper: String::new(),
                    composite_bps: "0".into(),
                    confidence_bps: 0,
                    timestamp: 0,
                    block: 0,
                    seen_at_unix: now_unix(),
                },
            );
        }
    }

    let mut market_stream = if let Some(addr) = market_addr {
        let f = Filter::new()
            .address(addr)
            .event_signature(JobAwarded::SIGNATURE_HASH)
            .from_block(from_block);
        Some(provider.subscribe_logs(&f).await?.into_stream())
    } else {
        None
    };

    let mut agent_stream = if let Some(addr) = agent_addr {
        let f = Filter::new()
            .address(addr)
            .event_signature(AgentRegistered::SIGNATURE_HASH)
            .from_block(from_block);
        Some(provider.subscribe_logs(&f).await?.into_stream())
    } else {
        None
    };

    let mut isfr_rate_stream = if !isfr_addrs.is_empty() {
        let f = Filter::new()
            .address(isfr_addrs.clone())
            .event_signature(RateSubmitted::SIGNATURE_HASH)
            .from_block(from_block);
        Some(provider.subscribe_logs(&f).await?.into_stream())
    } else {
        None
    };

    let mut isfr_range_stream = if !isfr_addrs.is_empty() {
        let f = Filter::new()
            .address(isfr_addrs.clone())
            .event_signature(RangeClosed::SIGNATURE_HASH)
            .from_block(from_block);
        Some(provider.subscribe_logs(&f).await?.into_stream())
    } else {
        None
    };

    let mut worker_register_stream = if let Some(addr) = worker_registry_addr {
        let f = Filter::new()
            .address(addr)
            .event_signature(WorkerRegistered::SIGNATURE_HASH)
            .from_block(from_block);
        Some(provider.subscribe_logs(&f).await?.into_stream())
    } else {
        None
    };

    let mut reputation_stream = if let Some(addr) = worker_registry_addr {
        let f = Filter::new()
            .address(addr)
            .event_signature(ReputationUpdated::SIGNATURE_HASH)
            .from_block(from_block);
        Some(provider.subscribe_logs(&f).await?.into_stream())
    } else {
        None
    };

    let mut slash_stream = if let Some(addr) = worker_registry_addr {
        let f = Filter::new()
            .address(addr)
            .event_signature(WorkerSlashed::SIGNATURE_HASH)
            .from_block(from_block);
        Some(provider.subscribe_logs(&f).await?.into_stream())
    } else {
        None
    };

    info!("daeji-indexer: subscribed");

    loop {
        tokio::select! {
            Some(log) = async {
                match market_stream.as_mut() {
                    Some(s) => s.next().await,
                    None => futures::future::pending().await,
                }
            } => {
                handle_job_awarded(log, &store).await;
            }
            Some(log) = async {
                match agent_stream.as_mut() {
                    Some(s) => s.next().await,
                    None => futures::future::pending().await,
                }
            } => {
                handle_agent_registered(log, &store).await;
            }
            Some(log) = async {
                match isfr_rate_stream.as_mut() {
                    Some(s) => s.next().await,
                    None => futures::future::pending().await,
                }
            } => {
                handle_isfr_rate_submitted(log, &store).await;
            }
            Some(log) = async {
                match isfr_range_stream.as_mut() {
                    Some(s) => s.next().await,
                    None => futures::future::pending().await,
                }
            } => {
                handle_isfr_range_closed(log, &store).await;
            }
            Some(log) = async {
                match worker_register_stream.as_mut() {
                    Some(s) => s.next().await,
                    None => futures::future::pending().await,
                }
            } => {
                handle_worker_registered(log, &store).await;
            }
            Some(log) = async {
                match reputation_stream.as_mut() {
                    Some(s) => s.next().await,
                    None => futures::future::pending().await,
                }
            } => {
                handle_reputation_updated(log, &store).await;
            }
            Some(log) = async {
                match slash_stream.as_mut() {
                    Some(s) => s.next().await,
                    None => futures::future::pending().await,
                }
            } => {
                handle_worker_slashed(log, &store).await;
            }
            else => break,
        }
    }
    Ok(())
}

async fn handle_agent_registered(log: AlloyRpcLog, store: &SharedStore) {
    let raw = AlloyLog::new(log.address(), log.topics().to_vec(), log.data().data.clone())
        .unwrap_or_else(|| AlloyLog::new_unchecked(log.address(), Vec::new(), Default::default()));
    match AgentRegistered::decode_log(&raw) {
        Ok(decoded) => {
            let entry = Agent {
                address: format!("{:#x}", decoded.agent),
                passport_hash: format!("0x{}", hex::encode(decoded.passportHash.0)),
                capabilities: decoded.capabilities.clone(),
                block: log.block_number.unwrap_or_default(),
                seen_at_unix: now_unix(),
            };
            let key = entry.address.to_ascii_lowercase();
            let mut s = store.write().await;
            let was_new = s.agents.insert(key, entry).is_none();
            info!(
                agent = %format!("{:#x}", decoded.agent),
                block = log.block_number.unwrap_or_default(),
                was_new,
                "AgentRegistered indexed"
            );
        }
        Err(err) => {
            error!(?err, "failed to decode AgentRegistered log");
        }
    }
}

async fn handle_job_awarded(log: AlloyRpcLog, store: &SharedStore) {
    let raw = AlloyLog::new(log.address(), log.topics().to_vec(), log.data().data.clone())
        .unwrap_or_else(|| AlloyLog::new_unchecked(log.address(), Vec::new(), Default::default()));
    match JobAwarded::decode_log(&raw) {
        Ok(decoded) => {
            let id_u256: U256 = decoded.id;
            let id = u64::try_from(id_u256).unwrap_or_else(|_| {
                warn!(id_hex = %id_u256, "job_id exceeds u64; truncating to lower 64 bits");
                id_u256.as_limbs()[0]
            });
            let room_id_bytes: FixedBytes<32> = decoded.roomId;
            let entry = Job {
                id,
                winners: decoded.winners.iter().map(|a| format!("{:#x}", a)).collect(),
                room_id: format!("0x{}", hex::encode(room_id_bytes.0)),
                block: log.block_number.unwrap_or_default(),
                seen_at_unix: now_unix(),
            };
            let mut s = store.write().await;
            let was_new = s.jobs.insert(id, entry).is_none();
            info!(
                id,
                winners_count = decoded.winners.len(),
                block = log.block_number.unwrap_or_default(),
                was_new,
                "JobAwarded indexed"
            );
        }
        Err(err) => {
            error!(?err, "failed to decode JobAwarded log");
        }
    }
}

async fn handle_worker_registered(log: AlloyRpcLog, store: &SharedStore) {
    let raw = AlloyLog::new(log.address(), log.topics().to_vec(), log.data().data.clone())
        .unwrap_or_else(|| AlloyLog::new_unchecked(log.address(), Vec::new(), Default::default()));
    match WorkerRegistered::decode_log(&raw) {
        Ok(decoded) => {
            // Fresh registration: reputation starts at SCALE/2 = 500_000 → bps 5000.
            let entry = Worker {
                address: format!("{:#x}", decoded.worker),
                bond: decoded.bond.to_string(),
                reputation: 500_000,
                reputation_bps: 5000,
                tier: WorkerTier::Standard,
                block: log.block_number.unwrap_or_default(),
                seen_at_unix: now_unix(),
            };
            let key = entry.address.to_ascii_lowercase();
            let mut s = store.write().await;
            s.workers.insert(key, entry);
            info!(
                worker = %format!("{:#x}", decoded.worker),
                bond = %decoded.bond,
                block = log.block_number.unwrap_or_default(),
                "WorkerRegistered indexed"
            );
        }
        Err(err) => error!(?err, "failed to decode WorkerRegistered log"),
    }
}

async fn handle_reputation_updated(log: AlloyRpcLog, store: &SharedStore) {
    let raw = AlloyLog::new(log.address(), log.topics().to_vec(), log.data().data.clone())
        .unwrap_or_else(|| AlloyLog::new_unchecked(log.address(), Vec::new(), Default::default()));
    match ReputationUpdated::decode_log(&raw) {
        Ok(decoded) => {
            let key = format!("{:#x}", decoded.worker).to_ascii_lowercase();
            let new_rep_u32 = u32::try_from(decoded.newRep).unwrap_or(u32::MAX);
            let new_bps = u16::try_from(new_rep_u32 / 100).unwrap_or(u16::MAX);
            let new_tier = WorkerTier::from_reputation(new_rep_u32);
            let mut s = store.write().await;
            if let Some(w) = s.workers.get_mut(&key) {
                w.reputation = new_rep_u32;
                w.reputation_bps = new_bps;
                w.tier = new_tier;
                w.block = log.block_number.unwrap_or_default();
                w.seen_at_unix = now_unix();
            } else {
                // ReputationUpdated for an unknown worker — defensively insert
                // with whatever info we have. Bond is 0 (we'll learn it from a
                // later WorkerRegistered or rely on contract reads at /workers/:addr).
                s.workers.insert(
                    key.clone(),
                    Worker {
                        address: format!("{:#x}", decoded.worker),
                        bond: "0".into(),
                        reputation: new_rep_u32,
                        reputation_bps: new_bps,
                        tier: new_tier,
                        block: log.block_number.unwrap_or_default(),
                        seen_at_unix: now_unix(),
                    },
                );
            }
            info!(
                worker = %format!("{:#x}", decoded.worker),
                old_rep = %decoded.oldRep,
                new_rep = %decoded.newRep,
                outcome = decoded.outcome,
                "ReputationUpdated indexed"
            );
        }
        Err(err) => error!(?err, "failed to decode ReputationUpdated log"),
    }
}

async fn handle_worker_slashed(log: AlloyRpcLog, store: &SharedStore) {
    let raw = AlloyLog::new(log.address(), log.topics().to_vec(), log.data().data.clone())
        .unwrap_or_else(|| AlloyLog::new_unchecked(log.address(), Vec::new(), Default::default()));
    match WorkerSlashed::decode_log(&raw) {
        Ok(decoded) => {
            let key = format!("{:#x}", decoded.worker).to_ascii_lowercase();
            let mut s = store.write().await;
            if let Some(w) = s.workers.get_mut(&key) {
                w.bond = decoded.newBond.to_string();
                w.block = log.block_number.unwrap_or_default();
                w.seen_at_unix = now_unix();
            }
            info!(
                worker = %format!("{:#x}", decoded.worker),
                reason_code = decoded.reasonCode,
                amount = %decoded.amount,
                new_bond = %decoded.newBond,
                "WorkerSlashed indexed"
            );
        }
        Err(err) => error!(?err, "failed to decode WorkerSlashed log"),
    }
}

async fn handle_isfr_rate_submitted(log: AlloyRpcLog, store: &SharedStore) {
    let raw = AlloyLog::new(log.address(), log.topics().to_vec(), log.data().data.clone())
        .unwrap_or_else(|| AlloyLog::new_unchecked(log.address(), Vec::new(), Default::default()));
    match RateSubmitted::decode_log(&raw) {
        Ok(decoded) => {
            let market = format!("{:#x}", log.address());
            let entry = IsfrRate {
                market: market.clone(),
                epoch_id: decoded.epochId,
                keeper: format!("{:#x}", decoded.keeper),
                composite_bps: decoded.compositeBps.to_string(),
                confidence_bps: decoded.confidenceBps,
                timestamp: decoded.timestamp,
                block: log.block_number.unwrap_or_default(),
                seen_at_unix: now_unix(),
            };
            let mut s = store.write().await;
            // Latest-wins: we only retain the most recent rate per market.
            s.isfr_rates.insert(market.to_ascii_lowercase(), entry);
            info!(
                market = %format!("{:#x}", log.address()),
                epoch = decoded.epochId,
                composite_bps = %decoded.compositeBps,
                confidence_bps = decoded.confidenceBps,
                block = log.block_number.unwrap_or_default(),
                "ISFR RateSubmitted indexed"
            );
        }
        Err(err) => {
            error!(?err, "failed to decode RateSubmitted log");
        }
    }
}

async fn handle_isfr_range_closed(log: AlloyRpcLog, store: &SharedStore) {
    let raw = AlloyLog::new(log.address(), log.topics().to_vec(), log.data().data.clone())
        .unwrap_or_else(|| AlloyLog::new_unchecked(log.address(), Vec::new(), Default::default()));
    match RangeClosed::decode_log(&raw) {
        Ok(decoded) => {
            let market = format!("{:#x}", log.address());
            let range_id = format!("0x{}", hex::encode(decoded.rangeId.0));
            let entry = IsfrRangeClose {
                market: market.clone(),
                range_id: range_id.clone(),
                range_start: decoded.rangeStart,
                range_end: decoded.rangeEnd,
                composite_bps: decoded.compositeBps.to_string(),
                voter_count: decoded.voterCount,
                block: log.block_number.unwrap_or_default(),
                seen_at_unix: now_unix(),
            };
            let mut s = store.write().await;
            s.isfr_ranges.insert(
                (market.to_ascii_lowercase(), range_id.to_ascii_lowercase()),
                entry,
            );
            info!(
                market = %format!("{:#x}", log.address()),
                range_id = %range_id,
                range_start = decoded.rangeStart,
                range_end = decoded.rangeEnd,
                voter_count = decoded.voterCount,
                "ISFR RangeClosed indexed"
            );
        }
        Err(err) => {
            error!(?err, "failed to decode RangeClosed log");
        }
    }
}

async fn health(State(store): State<SharedStore>) -> impl IntoResponse {
    let s = store.read().await;
    Json(serde_json::json!({
        "status": "ok",
        "agents": s.agents.len(),
        "jobs": s.jobs.len(),
        "isfr_markets": s.isfr_rates.len(),
        "isfr_ranges": s.isfr_ranges.len(),
    }))
}

/// Query params for `/agents`. Supports `?min_tier=Probation|Standard|Trusted|Elite`
/// for reputation-gated discovery; agent-tier lookup goes through `workers` map.
#[derive(Debug, Deserialize)]
struct AgentListQuery {
    min_tier: Option<String>,
}

async fn list_agents(
    State(store): State<SharedStore>,
    axum::extract::Query(q): axum::extract::Query<AgentListQuery>,
) -> Result<Json<Vec<Agent>>, StatusCode> {
    let min_tier = match q.min_tier.as_deref() {
        None => None,
        Some(s) => Some(s.parse::<WorkerTier>().map_err(|_| StatusCode::BAD_REQUEST)?),
    };

    let s = store.read().await;
    let mut out: Vec<Agent> = if let Some(min) = min_tier {
        // Filter: agent's address must have a Worker record with tier >= min.
        s.agents
            .values()
            .filter(|a| {
                s.workers
                    .get(&a.address.to_ascii_lowercase())
                    .map_or(false, |w| w.tier >= min)
            })
            .cloned()
            .collect()
    } else {
        s.agents.values().cloned().collect()
    };
    out.sort_by(|a, b| b.block.cmp(&a.block));
    Ok(Json(out))
}

async fn get_agent(
    State(store): State<SharedStore>,
    Path(address): Path<String>,
) -> Result<Json<Agent>, StatusCode> {
    let key = normalize_address(&address);
    let s = store.read().await;
    s.agents
        .get(&key)
        .cloned()
        .map(Json)
        .ok_or(StatusCode::NOT_FOUND)
}

async fn list_jobs(State(store): State<SharedStore>) -> impl IntoResponse {
    let s = store.read().await;
    let mut out: Vec<Job> = s.jobs.values().cloned().collect();
    out.sort_by(|a, b| b.id.cmp(&a.id));
    Json(out)
}

async fn get_job(
    State(store): State<SharedStore>,
    Path(id): Path<u64>,
) -> Result<Json<Job>, StatusCode> {
    let s = store.read().await;
    s.jobs.get(&id).cloned().map(Json).ok_or(StatusCode::NOT_FOUND)
}

async fn get_room(
    State(store): State<SharedStore>,
    Path(job_id): Path<u64>,
) -> Result<Json<RoomBinding>, StatusCode> {
    let s = store.read().await;
    let job = s.jobs.get(&job_id).cloned().ok_or(StatusCode::NOT_FOUND)?;
    let (slot_index, channel_id) = derive_slot_and_channel(job_id);
    Ok(Json(RoomBinding {
        job_id,
        room_id: job.room_id,
        slot_index,
        channel_id,
        winners: job.winners,
    }))
}

async fn list_isfr_markets(State(store): State<SharedStore>) -> impl IntoResponse {
    let s = store.read().await;
    let mut out: Vec<String> = s.isfr_rates.values().map(|r| r.market.clone()).collect();
    out.sort();
    out.dedup();
    Json(out)
}

async fn list_isfr_rates(State(store): State<SharedStore>) -> impl IntoResponse {
    let s = store.read().await;
    let mut out: Vec<IsfrRate> = s.isfr_rates.values().cloned().collect();
    out.sort_by(|a, b| b.block.cmp(&a.block));
    Json(out)
}

async fn get_isfr_rate(
    State(store): State<SharedStore>,
    Path(market): Path<String>,
) -> Result<Json<IsfrRate>, StatusCode> {
    let key = normalize_address(&market);
    let s = store.read().await;
    s.isfr_rates.get(&key).cloned().map(Json).ok_or(StatusCode::NOT_FOUND)
}

async fn list_isfr_ranges(State(store): State<SharedStore>) -> impl IntoResponse {
    let s = store.read().await;
    let mut out: Vec<IsfrRangeClose> = s.isfr_ranges.values().cloned().collect();
    out.sort_by(|a, b| b.block.cmp(&a.block));
    Json(out)
}

async fn list_workers(State(store): State<SharedStore>) -> impl IntoResponse {
    let s = store.read().await;
    let mut out: Vec<Worker> = s.workers.values().cloned().collect();
    out.sort_by(|a, b| b.reputation.cmp(&a.reputation));
    Json(out)
}

async fn get_worker(
    State(store): State<SharedStore>,
    Path(address): Path<String>,
) -> Result<Json<Worker>, StatusCode> {
    let key = normalize_address(&address);
    let s = store.read().await;
    s.workers.get(&key).cloned().map(Json).ok_or(StatusCode::NOT_FOUND)
}

async fn ingest_trading_fills(
    State(store): State<SharedStore>,
    Json(req): Json<TradingIngestRequest>,
) -> Result<Json<Vec<TraderSnapshot>>, StatusCode> {
    if req.fills.is_empty() {
        return Err(StatusCode::BAD_REQUEST);
    }
    let protocol_fee_bps = req.protocol_fee_bps.unwrap_or(1_000);
    if protocol_fee_bps > 10_000 {
        return Err(StatusCode::BAD_REQUEST);
    }

    let mut grouped: HashMap<String, Vec<TradingFill>> = HashMap::new();
    {
        let s = store.read().await;
        for fill in &req.fills {
            if fill.fill_id.is_empty()
                || fill.trader.is_empty()
                || fill.notional_usd <= 0.0
                || s.trading_fill_ids.contains_key(&fill.fill_id)
            {
                return Err(StatusCode::BAD_REQUEST);
            }
            grouped.entry(normalize_address(&fill.trader)).or_default().push(fill.clone());
        }
    }

    let mut snapshots = Vec::new();
    let mut s = store.write().await;
    for fill in &req.fills {
        s.trading_fill_ids.insert(fill.fill_id.clone(), true);
    }
    for (trader, fills) in grouped {
        let snapshot = build_trader_snapshot(&trader, fills, protocol_fee_bps);
        if let Some(worker) = s.workers.get_mut(&trader) {
            let delta: i32 = if snapshot.pnl_usd >= 0.0 { 25_000 } else { -25_000 };
            let next = (worker.reputation as i32 + delta).clamp(0, 1_000_000) as u32;
            worker.reputation = next;
            worker.reputation_bps = u16::try_from(next / 100).unwrap_or(u16::MAX);
            worker.tier = WorkerTier::from_reputation(next);
            worker.seen_at_unix = snapshot.last_seen_unix;
        }
        s.trader_snapshots.insert(trader.clone(), snapshot.clone());
        snapshots.push(snapshot);
    }
    snapshots.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal));
    Ok(Json(snapshots))
}

async fn list_trader_snapshots(State(store): State<SharedStore>) -> impl IntoResponse {
    let s = store.read().await;
    let mut out: Vec<TraderSnapshot> = s.trader_snapshots.values().cloned().collect();
    out.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal));
    Json(serde_json::json!({
        "traders": out,
        "count": out.len(),
        "source": "daeji-indexer"
    }))
}

async fn get_trader_snapshot(
    State(store): State<SharedStore>,
    Path(trader): Path<String>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    let key = normalize_address(&trader);
    let s = store.read().await;
    let snapshot = s.trader_snapshots.get(&key).cloned().ok_or(StatusCode::NOT_FOUND)?;
    Ok(Json(serde_json::json!({ "trader": snapshot })))
}

fn build_trader_snapshot(trader: &str, fills: Vec<TradingFill>, protocol_fee_bps: u16) -> TraderSnapshot {
    let pnl_usd: f64 = fills.iter().map(|f| f.pnl_usd).sum();
    let fees_usd: f64 = fills.iter().map(|f| f.fee_usd).sum();
    let notional_usd: f64 = fills.iter().map(|f| f.notional_usd).sum();
    let max_drawdown_bps = max_drawdown_bps(&fills);
    let fills_root = fills_root(&fills);
    let last_seen_unix = fills.iter().map(|f| f.timestamp_unix).max().unwrap_or_else(now_unix);
    let score = pnl_usd - fees_usd - (notional_usd * f64::from(max_drawdown_bps) / 10_000.0);
    let tier = if score >= 10_000.0 {
        "elite"
    } else if score >= 1_000.0 {
        "trusted"
    } else if score >= 0.0 {
        "standard"
    } else {
        "probation"
    }
    .to_string();
    let claim_id = format!("{:#x}", keccak256(format!("{trader}:{fills_root}:{last_seen_unix}").as_bytes()));
    let gross_earnings = if pnl_usd > 0.0 { (pnl_usd * 1_000_000.0) as u128 } else { 0 };
    TraderSnapshot {
        trader_id: trader.to_string(),
        score,
        tier,
        pnl_usd,
        fees_usd,
        max_drawdown_bps,
        fill_count: fills.len(),
        fills_root: fills_root.clone(),
        last_seen_unix,
        claim: TradingPnLClaim {
            claim_id,
            job_id: fills.iter().find_map(|f| f.job_id),
            trader: trader.to_string(),
            gross_earnings: gross_earnings.to_string(),
            protocol_fee_bps,
            fills_root,
            metadata_hash: format!("{:#x}", keccak256(format!("{pnl_usd}:{fees_usd}:{notional_usd}").as_bytes())),
        },
    }
}

fn max_drawdown_bps(fills: &[TradingFill]) -> u16 {
    let mut peak = 0.0_f64;
    let mut equity = 0.0_f64;
    let mut max_drawdown = 0.0_f64;
    for fill in fills {
        equity += fill.pnl_usd - fill.fee_usd;
        peak = peak.max(equity);
        if peak > 0.0 {
            max_drawdown = max_drawdown.max((peak - equity) / peak);
        }
    }
    (max_drawdown * 10_000.0).clamp(0.0, 10_000.0) as u16
}

fn fills_root(fills: &[TradingFill]) -> String {
    let mut ids: Vec<String> = fills.iter().map(|f| f.fill_id.clone()).collect();
    ids.sort();
    format!("{:#x}", keccak256(ids.join("|").as_bytes()))
}

/// Local re-derivation of the slot index + channel id. Mirrors the constants in
/// `daeji-chat::room` (POOL_SIZE = 64, SLOT_BASE = keccak("DAEJI_JOB_SLOT_V1")[..8] LE).
/// Lifted into the indexer rather than depending on `daeji-chat` so this crate
/// can be deployed standalone (no commonware dep tree).
fn derive_slot_and_channel(job_id: u64) -> (u32, u64) {
    const ROOM_DOMAIN: &[u8] = b"DAEJI_ROOM_V1";
    const JOB_SLOT_DOMAIN: &[u8] = b"DAEJI_JOB_SLOT_V1";
    const POOL_SIZE: u32 = 64;

    use alloy::primitives::keccak256;
    let mut packed = Vec::with_capacity(ROOM_DOMAIN.len() + 32);
    packed.extend_from_slice(ROOM_DOMAIN);
    let mut id_be = [0u8; 32];
    id_be[24..].copy_from_slice(&job_id.to_be_bytes());
    packed.extend_from_slice(&id_be);
    let room = keccak256(&packed);

    let mut slot_buf = [0u8; 4];
    slot_buf.copy_from_slice(&room[..4]);
    let slot_index = u32::from_le_bytes(slot_buf) % POOL_SIZE;

    let slot_base_hash = keccak256(JOB_SLOT_DOMAIN);
    let mut base_buf = [0u8; 8];
    base_buf.copy_from_slice(&slot_base_hash[..8]);
    let slot_base = u64::from_le_bytes(base_buf);

    (slot_index, slot_base.wrapping_add(slot_index as u64))
}

fn normalize_address(s: &str) -> String {
    s.trim().to_ascii_lowercase()
}

fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Cross-language parity: indexer's slot/channel derivation must match
    /// `daeji-chat::room::slot_for_chain_job` + `channel_id_for_slot`.
    /// Concrete value for job_id=7: room_id starts 0x349edc28...; first 4 bytes
    /// little-endian → 0x28dc9e34 → mod 64 = 0x14 (20).
    #[test]
    fn derive_slot_and_channel_matches_daeji_chat_for_7() {
        let (slot, _channel) = derive_slot_and_channel(7);
        // Reference value from daeji-chat tests: room_id_for_chain_job(7) =
        // 349edc281e8962dc4cdcb608f603ff0947711607f3d3412cea0b7e07e2899a90
        // → first 4 bytes LE: 0x28dc9e34 → mod 64.
        assert_eq!(slot, 0x28dc9e34u32 % 64);
    }

    #[test]
    fn derive_slot_in_pool_range_for_assorted_ids() {
        for id in [0u64, 1, 7, 42, 1_000, u64::MAX / 2, u64::MAX] {
            let (slot, _) = derive_slot_and_channel(id);
            assert!(slot < 64, "slot {} out of range for id {}", slot, id);
        }
    }

    #[test]
    fn derive_slot_is_deterministic() {
        let a = derive_slot_and_channel(42);
        let b = derive_slot_and_channel(42);
        assert_eq!(a, b);
    }

    #[test]
    fn normalize_address_lowercases_and_trims() {
        assert_eq!(
            normalize_address("  0xABcDef0123  "),
            "0xabcdef0123".to_string()
        );
    }

    #[test]
    fn worker_tier_thresholds_match_contract() {
        // Mirrors `WorkerRegistry.tier()` thresholds at lines 168-171 of
        // contracts-core/packages/agents/src/WorkerRegistry.sol.
        assert_eq!(WorkerTier::from_reputation(0), WorkerTier::Probation);
        assert_eq!(WorkerTier::from_reputation(349_999), WorkerTier::Probation);
        assert_eq!(WorkerTier::from_reputation(350_000), WorkerTier::Standard);
        assert_eq!(WorkerTier::from_reputation(549_999), WorkerTier::Standard);
        assert_eq!(WorkerTier::from_reputation(550_000), WorkerTier::Trusted);
        assert_eq!(WorkerTier::from_reputation(799_999), WorkerTier::Trusted);
        assert_eq!(WorkerTier::from_reputation(800_000), WorkerTier::Elite);
        assert_eq!(WorkerTier::from_reputation(1_000_000), WorkerTier::Elite);
    }

    #[test]
    fn worker_tier_ordering_is_correct_for_min_tier_filter() {
        // PartialOrd derive uses variant order. Filter logic is `tier >= min`.
        assert!(WorkerTier::Elite > WorkerTier::Trusted);
        assert!(WorkerTier::Trusted > WorkerTier::Standard);
        assert!(WorkerTier::Standard > WorkerTier::Probation);
        assert!(WorkerTier::Probation > WorkerTier::Unregistered);
    }

    #[test]
    fn worker_tier_parse_round_trip() {
        use std::str::FromStr as _;
        assert_eq!(
            WorkerTier::from_str("trusted").unwrap(),
            WorkerTier::Trusted
        );
        assert_eq!(WorkerTier::from_str("ELITE").unwrap(), WorkerTier::Elite);
        assert_eq!(
            WorkerTier::from_str("Probation").unwrap(),
            WorkerTier::Probation
        );
        assert!(WorkerTier::from_str("nonsense").is_err());
    }
}
