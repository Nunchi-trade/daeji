//! Chain-event watcher — runs as a background task inside `service::run_chat`.
//!
//! Subscribes to two event streams on a JSON-RPC WebSocket endpoint:
//!
//! - `AgentRegistry.AgentRegistered(address, bytes32, string)` — parses the
//!   `endpoint=URL` segment from the on-chain `capabilities` string, GETs that
//!   URL, verifies `keccak256(body) == passportHash`, extracts the
//!   `transport.pubkey` from the JSON-LD card, and writes a `Chain { ... }`
//!   record to the configured registry file. The agent runtime's existing
//!   200ms registry-poll loop picks the change up and calls `oracle.track`.
//!
//! - `MultiAgentMarket.JobAwarded(uint256, address[], bytes32)` — derives the
//!   commonware channel id locally, verifies parity with the on-chain emitted
//!   roomId, logs the binding. **Auto-join is deferred** to a future PR because
//!   commonware-p2p requires channels to be registered before `network.start()`,
//!   so dynamic channel registration on JobAwarded requires an architectural
//!   reshape (single chat channel + in-band routing, or a "lobby" channel
//!   approach). Tracked as Q-Open-6 in the canonical plan.

use std::{path::PathBuf, str::FromStr, time::Duration};

use alloy::{
    primitives::{Address, Bytes, FixedBytes, Log as AlloyLog, U256},
    providers::{Provider, ProviderBuilder, WsConnect},
    rpc::types::eth::{Filter, Log as AlloyRpcLog},
    sol,
    sol_types::SolEvent,
};
use futures::StreamExt;
use serde::{Deserialize, Serialize};
use tracing::{debug, error, info, warn};

use crate::{card, registry::Registry, room};

sol! {
    #[derive(Debug)]
    event JobAwarded(uint256 indexed id, address[] winners, bytes32 roomId);

    #[derive(Debug)]
    event AgentRegistered(address indexed agent, bytes32 passportHash, string capabilities);
}

/// Configuration for the chain-event watcher. Optional; included as
/// `Option<ChainConfig>` on `ChatConfig`. When present, `service::run_chat`
/// spawns a background task that runs `ChainWatcher::run`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChainConfig {
    /// JSON-RPC WebSocket endpoint (e.g., `ws://127.0.0.1:8545`).
    pub rpc_ws: String,

    /// `AgentRegistry` contract address (0x-hex). When set, watcher subscribes
    /// to `AgentRegistered` events and updates the registry file.
    #[serde(default)]
    pub agent_registry: Option<String>,

    /// `MultiAgentMarket` contract address (0x-hex). When set, watcher
    /// subscribes to `JobAwarded` events and logs channel bindings.
    #[serde(default)]
    pub market: Option<String>,

    /// Block to start scanning from. Defaults to the chain head at startup.
    #[serde(default)]
    pub from_block: Option<u64>,

    /// Operator's own EVM controller address (0x-hex). When `JobAwarded.winners`
    /// includes this address, watcher logs that the agent was awarded the job.
    /// Auto-join is deferred (Q-Open-6).
    #[serde(default)]
    pub my_controller: Option<String>,
}

#[derive(Debug, thiserror::Error)]
pub enum ChainWatcherError {
    #[error("invalid hex address: {0}")]
    InvalidAddress(String),
    #[error("WS connection failed: {0}")]
    Connect(String),
    #[error("subscription failed: {0}")]
    Subscribe(String),
}

/// Spawn the chain-event watcher. Runs forever (until the WS connection drops);
/// caller spawns this on a tokio task.
pub async fn run_chain_watcher(
    cfg: ChainConfig,
    registry_path: PathBuf,
) -> Result<(), ChainWatcherError> {
    let agent_registry_addr = cfg
        .agent_registry
        .as_deref()
        .map(Address::from_str)
        .transpose()
        .map_err(|e| ChainWatcherError::InvalidAddress(e.to_string()))?;
    let market_addr = cfg
        .market
        .as_deref()
        .map(Address::from_str)
        .transpose()
        .map_err(|e| ChainWatcherError::InvalidAddress(e.to_string()))?;
    let my_controller = cfg
        .my_controller
        .as_deref()
        .map(Address::from_str)
        .transpose()
        .map_err(|e| ChainWatcherError::InvalidAddress(e.to_string()))?;

    info!(
        rpc_ws = %cfg.rpc_ws,
        agent_registry = ?agent_registry_addr,
        market = ?market_addr,
        my_controller = ?my_controller,
        registry_path = %registry_path.display(),
        "chat-chain-watcher: connecting"
    );

    let provider = ProviderBuilder::new()
        .connect_ws(WsConnect::new(&cfg.rpc_ws))
        .await
        .map_err(|e| ChainWatcherError::Connect(e.to_string()))?;

    let chain_id = provider
        .get_chain_id()
        .await
        .map_err(|e| ChainWatcherError::Connect(e.to_string()))?;
    let head = provider
        .get_block_number()
        .await
        .map_err(|e| ChainWatcherError::Connect(e.to_string()))?;
    info!(chain_id, head, "chat-chain-watcher: connected");

    let from_block = cfg.from_block.unwrap_or(head);

    let mut market_stream_opt = if let Some(addr) = market_addr {
        let f = Filter::new()
            .address(addr)
            .event_signature(JobAwarded::SIGNATURE_HASH)
            .from_block(from_block);
        Some(
            provider
                .subscribe_logs(&f)
                .await
                .map_err(|e| ChainWatcherError::Subscribe(e.to_string()))?
                .into_stream(),
        )
    } else {
        None
    };

    let mut agent_stream_opt = if let Some(addr) = agent_registry_addr {
        let f = Filter::new()
            .address(addr)
            .event_signature(AgentRegistered::SIGNATURE_HASH)
            .from_block(from_block);
        Some(
            provider
                .subscribe_logs(&f)
                .await
                .map_err(|e| ChainWatcherError::Subscribe(e.to_string()))?
                .into_stream(),
        )
    } else {
        None
    };

    if market_stream_opt.is_none() && agent_stream_opt.is_none() {
        warn!("chat-chain-watcher: neither agent_registry nor market configured; nothing to watch");
        return Ok(());
    }

    info!("chat-chain-watcher: subscribed — waiting for events");

    let http = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .map_err(|e| ChainWatcherError::Connect(e.to_string()))?;

    loop {
        tokio::select! {
            Some(log) = async {
                match market_stream_opt.as_mut() {
                    Some(s) => s.next().await,
                    None => futures::future::pending().await,
                }
            } => {
                handle_job_awarded(log, my_controller);
            }
            Some(log) = async {
                match agent_stream_opt.as_mut() {
                    Some(s) => s.next().await,
                    None => futures::future::pending().await,
                }
            } => {
                handle_agent_registered(log, &http, &registry_path).await;
            }
            else => break,
        }
    }

    Ok(())
}

fn to_alloy_log(log: &AlloyRpcLog) -> AlloyLog {
    let topics = log.topics();
    let data = log.data().data.clone();
    AlloyLog::new(log.address(), topics.to_vec(), data)
        .unwrap_or_else(|| AlloyLog::new_unchecked(log.address(), Vec::new(), Bytes::new()))
}

fn handle_job_awarded(log: AlloyRpcLog, my_controller: Option<Address>) {
    let raw = to_alloy_log(&log);
    match JobAwarded::decode_log(&raw) {
        Ok(decoded) => {
            let id: U256 = decoded.id;
            let room_id: FixedBytes<32> = decoded.roomId;
            let winners = &decoded.winners;

            let job_id_u64 = u64::try_from(id).unwrap_or_else(|_| {
                warn!(id_hex = %id, "job_id exceeds u64; using truncated lower 64 bits");
                id.as_limbs()[0]
            });
            let derived_room = room::room_id_for_chain_job(job_id_u64);
            let channel_id = room::channel_id_from_room(&derived_room);

            let parity = if derived_room == room_id.0 { "✓" } else { "MISMATCH" };

            // Did we get awarded?
            let awarded_self = my_controller.map_or(false, |me| winners.iter().any(|w| *w == me));

            info!(
                job_id = job_id_u64,
                room_id_chain = %hex::encode(room_id),
                room_id_derived = %hex::encode(derived_room),
                parity,
                channel_id,
                winners_count = winners.len(),
                awarded_self,
                block = log.block_number.unwrap_or_default(),
                "JobAwarded → channel binding"
            );

            if awarded_self {
                warn!(
                    channel_id,
                    job_id = job_id_u64,
                    "JobAwarded includes this agent — auto-join is NOT YET IMPLEMENTED \
                     (commonware-p2p requires channel registration before network.start; \
                     architectural reshape pending — tracked as Q-Open-6)"
                );
            }
        }
        Err(err) => {
            error!(?err, "failed to decode JobAwarded log");
        }
    }
}

async fn handle_agent_registered(
    log: AlloyRpcLog,
    http: &reqwest::Client,
    registry_path: &std::path::Path,
) {
    let raw = to_alloy_log(&log);
    let decoded = match AgentRegistered::decode_log(&raw) {
        Ok(d) => d,
        Err(err) => {
            error!(?err, "failed to decode AgentRegistered log");
            return;
        }
    };

    let agent_addr = decoded.agent;
    let passport_hash: [u8; 32] = decoded.passportHash.0;
    let capabilities = decoded.capabilities.clone();
    let block = log.block_number.unwrap_or_default();

    let endpoint = match card::parse_endpoint(&capabilities) {
        Some(url) => url.to_string(),
        None => {
            warn!(
                agent = %agent_addr,
                block,
                "AgentRegistered without endpoint=URL in capabilities; skipping card fetch"
            );
            return;
        }
    };

    info!(agent = %agent_addr, block, endpoint = %endpoint, "AgentRegistered → fetching card");

    let body = match http.get(&endpoint).send().await {
        Ok(resp) => match resp.bytes().await {
            Ok(b) => b.to_vec(),
            Err(err) => {
                warn!(?err, endpoint = %endpoint, "failed to read card body");
                return;
            }
        },
        Err(err) => {
            warn!(?err, endpoint = %endpoint, "failed to GET card");
            return;
        }
    };

    let verified_card = match card::verify_card(&body, &passport_hash) {
        Ok(c) => c,
        Err(err) => {
            warn!(
                ?err,
                agent = %agent_addr,
                expected_passport_hash = %hex::encode(passport_hash),
                "card verification failed; not adding to registry"
            );
            return;
        }
    };

    let pubkey = match verified_card.transport_pubkey_bytes() {
        Ok(p) => p,
        Err(err) => {
            warn!(?err, "transport pubkey decode failed");
            return;
        }
    };

    let mut reg = Registry::load(registry_path).unwrap_or_else(|err| {
        debug!(?err, "registry not loadable; starting empty");
        Registry::empty()
    });

    let added = reg.add_chain(
        format!("{:#x}", agent_addr),
        format!("0x{}", hex::encode(pubkey)),
        capabilities,
        endpoint,
    );

    if !added {
        debug!(agent = %agent_addr, "agent already in registry; no-op");
        return;
    }

    if let Err(err) = reg.save(registry_path) {
        error!(?err, "failed to save registry");
        return;
    }

    info!(
        agent = %agent_addr,
        epoch = reg.epoch,
        transport_pubkey = %hex::encode(pubkey),
        "AgentRegistered → Chain record added to registry"
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chain_config_serde_round_trip() {
        let cfg = ChainConfig {
            rpc_ws: "ws://127.0.0.1:8545".into(),
            agent_registry: Some("0x5FbDB2315678afecb367f032d93F642f64180aa3".into()),
            market: Some("0xe7f1725E7734CE288F8367e1Bb143E90bb3F0512".into()),
            from_block: Some(0),
            my_controller: Some("0xf39Fd6e51aad88F6F4ce6aB8827279cfFFb92266".into()),
        };
        let json = serde_json::to_string(&cfg).unwrap();
        let parsed: ChainConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.rpc_ws, "ws://127.0.0.1:8545");
        assert_eq!(parsed.from_block, Some(0));
    }

    #[test]
    fn chain_config_omits_optionals() {
        let json = r#"{ "rpc_ws": "ws://localhost:8545" }"#;
        let parsed: ChainConfig = serde_json::from_str(json).unwrap();
        assert!(parsed.agent_registry.is_none());
        assert!(parsed.market.is_none());
        assert!(parsed.from_block.is_none());
    }
}
