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
    primitives::{Address, FixedBytes, Log as AlloyLog, U256},
    providers::{Provider, ProviderBuilder, WsConnect},
    rpc::types::eth::{Filter, Log as AlloyRpcLog},
    sol,
    sol_types::SolEvent,
};
use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    routing::get,
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

#[derive(Default)]
struct Store {
    agents: HashMap<String, Agent>, // 0x-lower address → Agent
    jobs: HashMap<u64, Job>,        // chain id → Job
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
        bind = %cli.bind,
        "daeji-indexer: starting"
    );

    let store: SharedStore = Arc::new(RwLock::new(Store::default()));

    let agent_addr = parse_optional_address("agent_registry", cli.agent_registry.as_deref())?;
    let market_addr = parse_optional_address("market", cli.market.as_deref())?;
    if agent_addr.is_none() && market_addr.is_none() {
        warn!("daeji-indexer: neither --agent-registry nor --market set; HTTP server will only return empty results");
    }

    let watch_store = store.clone();
    let watch_rpc = cli.rpc_ws.clone();
    let watch_from = cli.from_block;
    tokio::spawn(async move {
        if let Err(err) =
            run_chain_watcher(watch_rpc, agent_addr, market_addr, watch_from, watch_store).await
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

async fn health(State(store): State<SharedStore>) -> impl IntoResponse {
    let s = store.read().await;
    Json(serde_json::json!({
        "status": "ok",
        "agents": s.agents.len(),
        "jobs": s.jobs.len(),
    }))
}

async fn list_agents(State(store): State<SharedStore>) -> impl IntoResponse {
    let s = store.read().await;
    let mut out: Vec<Agent> = s.agents.values().cloned().collect();
    out.sort_by(|a, b| b.block.cmp(&a.block));
    Json(out)
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
}
