//! Chat service entry point — bolted into the kora chain binary.
//!
//! `run_chat(ctx, config)` is the function kora calls when `--enable-chat` is set.
//! It builds a commonware-p2p `discovery::Network` on a configurable port (separate
//! from kora's consensus network for v1 — see canonical-plan §13 for the rationale),
//! tracks the authorized peer set from a registry file, and pre-registers the lobby
//! channel + a fixed pool of slot channels per canonical-plan §14.
//!
//! Per-job state is populated dynamically:
//! - `LobbyMessage::JobAnnounce` arrives on the lobby → unwrap room key → insert
//!   into `ActiveJobs`. Subsequent slot traffic for that job is AEAD-decrypted using
//!   the per-job room key (room id bound as AAD).
//! - `LobbyMessage::JobConcluded` removes the entry, freeing the slot for reuse.
//!
//! Multiple jobs that hash to the same slot share the channel; the AEAD tag
//! disambiguates them at the application layer (try-decrypt against each active
//! job's room key; first success routes to that job's handler, all-fail discards).

use std::{
    collections::HashMap,
    net::{IpAddr, Ipv4Addr, SocketAddr},
    path::PathBuf,
    str::FromStr,
    sync::{Arc, Mutex},
    time::Duration,
};

use commonware_cryptography::{ed25519, Signer as _};
use commonware_p2p::{
    authenticated::discovery::{self, Config as DiscoveryConfig},
    Manager as _, Receiver, Recipients, Sender,
};
use commonware_runtime::{Clock, Metrics, Quota, Spawner};
use commonware_utils::NZU32;
use serde::{Deserialize, Serialize};
use tracing::{debug, info, warn};

use crate::{
    chain::ChainConfig,
    lobby::{LobbyMessage, RoomKeyWrap},
    messages::RoomMessage,
    registry::Registry,
    room,
};

/// Application namespace for replay protection. Distinct from kora's consensus
/// namespace so chat traffic can never collide with consensus traffic at the
/// signature-verification layer.
const APPLICATION_NAMESPACE: &[u8] = b"_DAEJI_CHAT_V1";

/// Maximum size of a single chat message in bytes (8 KiB).
const MAX_MESSAGE_SIZE: u32 = 8 * 1024;

/// Per-channel message backlog before back-pressure kicks in.
const MAX_BACKLOG: usize = 256;

/// How often the registry watcher re-reads the registry file.
const REGISTRY_POLL_INTERVAL: Duration = Duration::from_millis(200);

/// Configuration for the chat service. Read from kora's NodeConfig at startup.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatConfig {
    /// Whether to start the chat service. Default: false.
    #[serde(default)]
    pub enabled: bool,

    /// ed25519 seed for this agent's transport identity. POC shortcut — production
    /// will load a real keystore.
    pub me_seed: u64,

    /// Local TCP port for the chat mesh. Distinct from kora's consensus port.
    pub bind_port: u16,

    /// Bootstrappers in `<seed>@<host:port>` form.
    #[serde(default)]
    pub bootstrappers: Vec<String>,

    /// Path to the authorized-peer registry file. Polled every 200ms.
    pub registry_path: PathBuf,

    /// Pre-seeded active jobs. Useful for demos / smoke tests where there's no
    /// lobby coordinator yet driving JobAnnounce. Each entry activates a slot
    /// in the per-job state map at startup.
    #[serde(default)]
    pub seed_jobs: Vec<SeedJob>,

    /// When set (with non-empty `seed_jobs`), the service runs a demo driver that
    /// sends Hello → Status → Final on the FIRST seed job's slot
    /// `drive_after_secs` after start. Lets the existing 3-agent smoke test keep
    /// working without a lobby coordinator.
    #[serde(default)]
    pub drive: bool,

    /// Seconds after startup at which the demo driver fires (only with `drive`).
    #[serde(default = "default_drive_after")]
    pub drive_after_secs: u64,

    /// Optional chain-event watcher config. When set, `run_chat` spawns a
    /// background task that subscribes to `AgentRegistry.AgentRegistered` and
    /// `MultiAgentMarket.JobAwarded` events on the configured WS RPC. The
    /// AgentRegistered handler verifies the off-chain card via keccak +
    /// updates the registry file (which the agent's existing 200ms poller
    /// then picks up to refresh `oracle.track`). JobAwarded auto-join is
    /// driven via the lobby listener: the chain watcher logs awareness; the
    /// matching `JobAnnounce` on the lobby channel actually activates the slot.
    #[serde(default)]
    pub chain: Option<ChainConfig>,
}

/// One pre-seeded active job. Used for demos / smoke tests; production agents
/// populate `ActiveJobs` from `LobbyMessage::JobAnnounce` instead.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SeedJob {
    /// Decimal string for u64 portability with on-chain ids.
    pub job_id: String,
    /// 32-byte hex room key (pre-shared in v1; replaced by ECDH in PR-Daeji-F).
    pub room_key_hex: String,
}

const fn default_drive_after() -> u64 {
    3
}

#[derive(Debug, thiserror::Error)]
pub enum ChatServiceError {
    #[error("chat is disabled in config")]
    Disabled,
    #[error("invalid me_seed encoding")]
    InvalidSeed,
    #[error("invalid bootstrapper spec: {0}")]
    InvalidBootstrapper(String),
    #[error("invalid room key: must be 32 bytes of hex (64 chars)")]
    InvalidRoomKey,
    #[error("invalid seed_job.job_id: must be a u64 decimal string ({0})")]
    InvalidSeedJobId(String),
    #[error("registry load failed: {0}")]
    Registry(#[from] std::io::Error),
}

/// Per-active-job state held by the service. Indexed by the chain job id (u64)
/// in `ActiveJobs`; the slot index is recomputed from the job id on demand.
#[derive(Clone)]
struct ActiveJob {
    job_id: u64,
    room_id: [u8; 32],
    slot_index: u32,
    room_key: [u8; 32],
}

/// Shared state: chain job id → ActiveJob. Held behind a sync Mutex; all
/// critical sections are short (insert/remove/clone) so no async locking
/// is needed. Cloned cheaply for hand-off into spawned tasks.
type ActiveJobs = Arc<Mutex<HashMap<u64, ActiveJob>>>;

/// Trait alias bundling all the runtime bounds [`run_chat`] needs. Lets external
/// callers (e.g. the [`crate::supervisor::run_chat_supervised`] wrapper) write
/// `C: SupervisedContext` instead of repeating the seven-trait bound.
pub trait SupervisedContext:
    Spawner
    + Metrics
    + Clock
    + commonware_runtime::BufferPooler
    + commonware_runtime::Network
    + commonware_runtime::Resolver
    + rand_core::CryptoRngCore
{
}

impl<T> SupervisedContext for T where
    T: Spawner
        + Metrics
        + Clock
        + commonware_runtime::BufferPooler
        + commonware_runtime::Network
        + commonware_runtime::Resolver
        + rand_core::CryptoRngCore
{
}

/// Run the chat service. Builds a commonware-p2p network, tracks the registry-driven
/// peer set, pre-registers the lobby channel + 64 slot channels, spawns a lobby
/// listener (handles `LobbyMessage::JobAnnounce` → activate slot for the job) and
/// 64 slot try-decrypt loops, then runs forever.
///
/// kora calls this from `LegacyNodeService.run_with_context` (or equivalent) as
/// a spawned tokio task when `--enable-chat` is set. Wrap in
/// [`crate::supervisor::run_chat_supervised`] for retry-on-error semantics.
pub async fn run_chat<C>(context: C, config: ChatConfig) -> Result<(), ChatServiceError>
where
    C: SupervisedContext,
{
    if !config.enabled {
        return Err(ChatServiceError::Disabled);
    }

    // Runtime kill switch (canonical-plan §19 B2.3): an operator can disable
    // chat without touching config or recompiling by setting DAEJI_CHAT_DISABLED
    // to a truthy value. The check here gates startup; the supervisor wrapper
    // (PR-Daeji-G follow-up) will re-check on each respawn so SIGUSR1 + env-var
    // flip can disable a running chat without restarting kora.
    if crate::supervisor::disabled_via_env() {
        info!(
            env_var = crate::supervisor::DISABLE_ENV_VAR,
            "chat: disabled via env var; not starting"
        );
        return Err(ChatServiceError::Disabled);
    }

    let signer = ed25519::PrivateKey::from_seed(config.me_seed);
    info!(
        seed = config.me_seed,
        port = config.bind_port,
        key = ?signer.public_key(),
        "chat: loaded signer"
    );

    let bootstrappers = parse_bootstrappers(&config.bootstrappers)?;
    let me_pubkey_hex = format!("0x{}", hex::encode(signer.public_key().as_ref()));

    let p2p_cfg = DiscoveryConfig::local(
        signer.clone(),
        APPLICATION_NAMESPACE,
        SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), config.bind_port),
        SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), config.bind_port),
        bootstrappers,
        MAX_MESSAGE_SIZE,
    );

    let lobby_channel_id = room::lobby_channel_id();
    let slot_pool_base = room::slot_pool_base();
    info!(
        lobby_channel_id,
        slot_pool_base,
        pool_size = room::POOL_SIZE,
        "chat: lobby + slot pool channel ids derived"
    );

    let (mut network, mut oracle) =
        discovery::Network::new(context.with_label("chat-network"), p2p_cfg);

    // Initial peer-set from the registry file.
    let initial = match Registry::load(&config.registry_path) {
        Ok(r) => r,
        Err(err) => {
            warn!(?err, registry = %config.registry_path.display(), "chat: registry load failed; starting empty");
            Registry::empty()
        }
    };
    info!(epoch = initial.epoch, "chat: initial peer-set");
    oracle.track(initial.epoch, initial.pubkey_set()).await;

    // Pre-register lobby + every slot channel before network.start(). commonware-p2p
    // requires this — registrations after start are rejected.
    let (lobby_sender, lobby_receiver) =
        network.register(lobby_channel_id, Quota::per_second(NZU32!(32)), MAX_BACKLOG);

    // Compute the demo driver's slot up-front so we can capture the matching slot's
    // sender as we iterate. Returns None when drive=false or seed_jobs is empty.
    let driver_target = compute_driver_target(&config)?;

    // Per-job state — shared between the lobby listener (writer) and slot loops (readers).
    let active: ActiveJobs = Arc::new(Mutex::new(HashMap::new()));
    seed_active_jobs(&active, &config.seed_jobs)?;
    {
        let snapshot = active.lock().expect("active poisoned");
        info!(seeded_jobs = snapshot.len(), "chat: ActiveJobs initialized");
    }

    // Register each slot channel + spawn its receiver loop inline. No Vec<Sender>
    // needed because (a) slot loops only consume messages, (b) the demo driver only
    // needs the one sender for its target slot.
    let mut driver_sender = None;
    for slot in 0..room::POOL_SIZE {
        let cid = room::channel_id_for_slot(slot);
        let (tx, rx) = network.register(cid, Quota::per_second(NZU32!(64)), MAX_BACKLOG);

        let active_for_slot = active.clone();
        let me_for_slot = me_pubkey_hex.clone();
        context.with_label("chat-slot").spawn(move |_| async move {
            run_slot_loop(slot, rx, active_for_slot, me_for_slot).await;
        });

        if driver_target.as_ref().map(|t| t.slot) == Some(slot) {
            driver_sender = Some(tx);
        }
    }
    info!(
        registered = 1 + room::POOL_SIZE as usize,
        "chat: pre-registered lobby + slot pool channels"
    );

    // Background task: poll the registry file and refresh oracle.track on epoch change.
    let watcher_ctx = context.clone();
    let mut watcher_oracle = oracle.clone();
    let watcher_path = config.registry_path.clone();
    let mut last_epoch = initial.epoch;
    context
        .with_label("chat-registry-watcher")
        .spawn(move |_| async move {
            loop {
                watcher_ctx.sleep(REGISTRY_POLL_INTERVAL).await;
                match Registry::load(&watcher_path) {
                    Ok(reg) if reg.epoch != last_epoch => {
                        info!(
                            old_epoch = last_epoch,
                            new_epoch = reg.epoch,
                            "chat: registry epoch advanced — refreshing oracle peer-set"
                        );
                        watcher_oracle.track(reg.epoch, reg.pubkey_set()).await;
                        last_epoch = reg.epoch;
                    }
                    Ok(_) => {}
                    Err(err) => {
                        debug!(?err, "chat: registry poll error (transient?)");
                    }
                }
            }
        });

    // Optional chain-event watcher.
    if let Some(chain_cfg) = config.chain.clone() {
        let chain_registry_path = config.registry_path.clone();
        info!(rpc_ws = %chain_cfg.rpc_ws, "chat: spawning chain-event watcher");
        tokio::spawn(async move {
            if let Err(err) =
                crate::chain::run_chain_watcher(chain_cfg, chain_registry_path).await
            {
                tracing::error!(?err, "chat: chain watcher exited with error");
            }
        });
    }

    // Lobby listener: consumes LobbyMessage, mutates ActiveJobs.
    {
        let active = active.clone();
        let me_hex = me_pubkey_hex.clone();
        context.with_label("chat-lobby").spawn(move |_| async move {
            run_lobby_loop(lobby_receiver, active, me_hex).await;
        });
    }

    // Optional demo driver — fires Hello/Status/Final on the seed job's slot after
    // a delay so peers have time to discover. Preserves the existing 3-agent
    // smoke test path without needing a lobby coordinator.
    if let (Some(target), Some(mut sender)) = (driver_target.clone(), driver_sender) {
        let drive_delay = Duration::from_secs(config.drive_after_secs);
        let me_hex = me_pubkey_hex.clone();
        let driver_ctx = context.clone();
        let room_id = target.room_id;
        let room_key = target.room_key;
        context.with_label("chat-driver").spawn(move |_| async move {
            driver_ctx.sleep(drive_delay).await;
            drive_sequence(&mut sender, &room_key, &room_id, &me_hex).await;
        });
    } else if config.drive {
        warn!("chat: drive=true but seed_jobs is empty — driver disabled");
    }

    // lobby_sender is reserved for future RoomJoined replies; not yet wired.
    drop(lobby_sender);

    let network_handler = network.start();
    // run_chat returns when the network shuts down. The spawned tasks are detached;
    // their lifetimes are bound to the runtime's spawn scope. For long-running deploy
    // we just await the handler.
    let _ = network_handler.await;
    Ok(())
}

/// Read a `LobbyMessage` stream off the lobby channel. Mutates `ActiveJobs` on
/// `JobAnnounce` (activate) and `JobConcluded` (deactivate).
async fn run_lobby_loop<R: Receiver>(
    mut receiver: R,
    active: ActiveJobs,
    me_pubkey_hex: String,
) {
    info!("chat: lobby listener started");
    let me_lower = me_pubkey_hex.to_ascii_lowercase();

    loop {
        let (peer, wire) = match receiver.recv().await {
            Ok(v) => v,
            Err(err) => {
                debug!(?err, "chat: lobby receiver error");
                continue;
            }
        };
        let peer_hex = hex::encode(peer.as_ref());
        let msg: LobbyMessage = match serde_json::from_slice(wire.as_ref()) {
            Ok(m) => m,
            Err(err) => {
                warn!(peer = %short(&peer_hex), ?err, "chat: malformed lobby message");
                continue;
            }
        };

        match msg {
            LobbyMessage::JobAnnounce {
                job_id,
                slot_index,
                participants: _,
                room_key_wraps,
                announced_at_block,
                coordinator,
            } => {
                let job_id_u64 = match parse_job_id_u64(&job_id) {
                    Ok(v) => v,
                    Err(err) => {
                        warn!(job_id = %job_id, ?err, "chat: JobAnnounce with non-u64 job_id");
                        continue;
                    }
                };
                let derived_slot = room::slot_for_chain_job(job_id_u64);
                if derived_slot != slot_index {
                    warn!(
                        job_id_u64,
                        announced_slot = slot_index,
                        derived_slot,
                        "chat: JobAnnounce slot mismatch — dropping"
                    );
                    continue;
                }

                let my_wrap = room_key_wraps
                    .iter()
                    .find(|w| w.recipient_pubkey_hex.eq_ignore_ascii_case(&me_pubkey_hex)
                        || w.recipient_pubkey_hex.eq_ignore_ascii_case(&me_lower));
                let Some(my_wrap) = my_wrap else {
                    debug!(
                        job_id_u64,
                        coordinator = %short(&coordinator),
                        announced_at_block,
                        "chat: JobAnnounce — not addressed to me; ignoring"
                    );
                    continue;
                };

                let room_key = match unwrap_room_key(my_wrap) {
                    Ok(k) => k,
                    Err(err) => {
                        warn!(job_id_u64, ?err, "chat: room_key unwrap failed; skipping");
                        continue;
                    }
                };
                let room_id = room::room_id_for_chain_job(job_id_u64);
                let entry = ActiveJob {
                    job_id: job_id_u64,
                    room_id,
                    slot_index,
                    room_key,
                };

                let mut map = active.lock().expect("active poisoned");
                let was_new = map.insert(job_id_u64, entry).is_none();
                drop(map);

                info!(
                    job_id_u64,
                    slot_index,
                    coordinator = %short(&coordinator),
                    announced_at_block,
                    was_new,
                    "chat: ActiveJobs += JobAnnounce"
                );
            }
            LobbyMessage::JobConcluded { job_id, slot_index } => {
                let Ok(job_id_u64) = parse_job_id_u64(&job_id) else {
                    continue;
                };
                let mut map = active.lock().expect("active poisoned");
                let removed = map.remove(&job_id_u64).is_some();
                drop(map);
                if removed {
                    info!(
                        job_id_u64,
                        slot_index, "chat: ActiveJobs -= JobConcluded"
                    );
                }
            }
            LobbyMessage::RoomJoined {
                job_id,
                passport_id,
                signature_over_room_id: _,
            } => {
                debug!(job_id, passport_id, "chat: lobby RoomJoined");
            }
            LobbyMessage::MiningClaim {
                job_id,
                slot_index,
                agent,
                branch_id,
                claim_proof_hash,
            } => {
                debug!(
                    job_id,
                    slot_index,
                    agent = %short(&agent),
                    branch_id = ?branch_id,
                    claim_proof_hash = %claim_proof_hash,
                    "chat: lobby MiningClaim"
                );
            }
        }
    }
}

/// Read raw bytes off one slot channel. For each frame, try AEAD-decrypt against
/// every active job currently mapped to this slot. First success → parse as
/// `RoomMessage` + log; all-fail → silently discard.
async fn run_slot_loop<R: Receiver>(
    slot: u32,
    mut receiver: R,
    active: ActiveJobs,
    me_pubkey_hex: String,
) {
    info!(slot, "chat: slot loop started");
    loop {
        let (peer, wire) = match receiver.recv().await {
            Ok(v) => v,
            Err(err) => {
                debug!(slot, ?err, "chat: slot receiver error");
                continue;
            }
        };
        let peer_hex = hex::encode(peer.as_ref());

        // Snapshot the active jobs for this slot. Lock is brief; we don't hold
        // it across the AEAD decrypt loop.
        let candidates: Vec<ActiveJob> = {
            let map = active.lock().expect("active poisoned");
            map.values()
                .filter(|j| j.slot_index == slot)
                .cloned()
                .collect()
        };
        if candidates.is_empty() {
            debug!(
                slot,
                peer = %short(&peer_hex),
                "chat: slot frame received but no active jobs — discarding"
            );
            continue;
        }

        let mut routed = false;
        for job in &candidates {
            if let Some(plaintext) = room::decrypt(&job.room_key, &job.room_id, wire.as_ref()) {
                let msg: RoomMessage = match serde_json::from_slice(&plaintext) {
                    Ok(m) => m,
                    Err(err) => {
                        warn!(
                            slot,
                            job_id = job.job_id,
                            peer = %short(&peer_hex),
                            ?err,
                            "chat: malformed RoomMessage after AEAD decrypt"
                        );
                        continue;
                    }
                };
                info!(
                    slot,
                    job_id = job.job_id,
                    peer = %short(&peer_hex),
                    me = %short(&me_pubkey_hex),
                    kind = msg.label(),
                    "chat: rx"
                );
                routed = true;
                break;
            }
        }
        if !routed {
            debug!(
                slot,
                peer = %short(&peer_hex),
                candidates = candidates.len(),
                "chat: slot frame did not decrypt for any active job — discarding"
            );
        }
    }
}

async fn drive_sequence<S: Sender>(
    sender: &mut S,
    room_key: &[u8; 32],
    room: &[u8; 32],
    me_hex: &str,
) {
    info!(me = %short(me_hex), "chat: driver starting");
    send_room(
        sender,
        room_key,
        room,
        &RoomMessage::Hello {
            from_pubkey_hex: me_hex.to_string(),
            wall_clock_ms: now_ms(),
        },
    )
    .await;
    spin_yield(50).await;
    send_room(
        sender,
        room_key,
        room,
        &RoomMessage::Status {
            from_pubkey_hex: me_hex.to_string(),
            phase: "executing".into(),
            eta_blocks: 5,
        },
    )
    .await;
    spin_yield(50).await;
    send_room(
        sender,
        room_key,
        room,
        &RoomMessage::Final {
            from_pubkey_hex: me_hex.to_string(),
            result_hash_hex: "deadbeef".into(),
            signature_hex: "feedface".into(),
        },
    )
    .await;
}

async fn send_room<S: Sender>(
    sender: &mut S,
    room_key: &[u8; 32],
    room: &[u8; 32],
    msg: &RoomMessage,
) {
    let plaintext = serde_json::to_vec(msg).expect("serialize");
    let wire = room::encrypt(room_key, room, &plaintext);
    match sender.send(Recipients::All, wire, false).await {
        Ok(reached) if !reached.is_empty() => {
            info!(kind = msg.label(), reached = reached.len(), "chat: tx");
        }
        Ok(_) => {
            warn!(kind = msg.label(), "chat: tx — no peers reached");
        }
        Err(err) => {
            warn!(kind = msg.label(), ?err, "chat: tx failed");
        }
    }
}

async fn spin_yield(hops: usize) {
    for _ in 0..hops {
        futures::future::ready(()).await;
    }
}

fn seed_active_jobs(active: &ActiveJobs, seeds: &[SeedJob]) -> Result<(), ChatServiceError> {
    let mut map = active.lock().expect("active poisoned");
    for seed in seeds {
        let job_id = parse_job_id_u64(&seed.job_id)?;
        let room_key = parse_room_key(&seed.room_key_hex)?;
        let room_id = room::room_id_for_chain_job(job_id);
        let slot_index = room::slot_for_chain_job(job_id);
        map.insert(
            job_id,
            ActiveJob {
                job_id,
                room_id,
                slot_index,
                room_key,
            },
        );
    }
    Ok(())
}

fn parse_job_id_u64(s: &str) -> Result<u64, ChatServiceError> {
    s.parse::<u64>()
        .map_err(|_| ChatServiceError::InvalidSeedJobId(s.to_string()))
}

fn parse_bootstrappers(
    raw: &[String],
) -> Result<Vec<(ed25519::PublicKey, commonware_p2p::Ingress)>, ChatServiceError> {
    raw.iter()
        .map(|s| {
            let (seed_str, addr_str) = s
                .split_once('@')
                .ok_or_else(|| ChatServiceError::InvalidBootstrapper(s.clone()))?;
            let seed: u64 = seed_str
                .parse()
                .map_err(|_| ChatServiceError::InvalidBootstrapper(s.clone()))?;
            let addr = SocketAddr::from_str(addr_str)
                .map_err(|_| ChatServiceError::InvalidBootstrapper(s.clone()))?;
            Ok((ed25519::PrivateKey::from_seed(seed).public_key(), addr.into()))
        })
        .collect()
}

fn parse_room_key(hex_str: &str) -> Result<[u8; 32], ChatServiceError> {
    let trimmed = hex_str
        .trim()
        .strip_prefix("0x")
        .or_else(|| hex_str.trim().strip_prefix("0X"))
        .unwrap_or_else(|| hex_str.trim());
    let raw = hex::decode(trimmed).map_err(|_| ChatServiceError::InvalidRoomKey)?;
    if raw.len() != 32 {
        return Err(ChatServiceError::InvalidRoomKey);
    }
    let mut out = [0u8; 32];
    out.copy_from_slice(&raw);
    Ok(out)
}

fn unwrap_room_key(wrap: &RoomKeyWrap) -> Result<[u8; 32], ChatServiceError> {
    // v1: ciphertext field is the plaintext 32-byte room key as hex (PRE-handshake
    // per canonical-plan §14). PR-Daeji-F replaces this with X25519 ECDH-derived
    // AEAD ciphertext + nonce decode + decrypt against my x25519 secret.
    parse_room_key(&wrap.ciphertext_hex)
}

fn short(hex_pk: &str) -> String {
    if hex_pk.len() < 8 {
        return hex_pk.to_string();
    }
    format!("{}..{}", &hex_pk[..4], &hex_pk[hex_pk.len() - 4..])
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// Demo-driver target: the first seed job's room id + room key + slot. Computed
/// up-front so the slot-registration loop can capture the matching slot's sender.
#[derive(Clone)]
struct DriverTarget {
    slot: u32,
    room_id: [u8; 32],
    room_key: [u8; 32],
}

fn compute_driver_target(config: &ChatConfig) -> Result<Option<DriverTarget>, ChatServiceError> {
    if !config.drive {
        return Ok(None);
    }
    let Some(seed) = config.seed_jobs.first() else {
        return Ok(None);
    };
    let job_id = parse_job_id_u64(&seed.job_id)?;
    let room_key = parse_room_key(&seed.room_key_hex)?;
    Ok(Some(DriverTarget {
        slot: room::slot_for_chain_job(job_id),
        room_id: room::room_id_for_chain_job(job_id),
        room_key,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_serde_round_trip() {
        let cfg = ChatConfig {
            enabled: true,
            me_seed: 1,
            bind_port: 4101,
            bootstrappers: vec!["1@127.0.0.1:4101".into()],
            registry_path: PathBuf::from("/tmp/registry.json"),
            seed_jobs: vec![SeedJob {
                job_id: "42".into(),
                room_key_hex: "00".repeat(32),
            }],
            drive: false,
            drive_after_secs: 3,
            chain: None,
        };
        let json = serde_json::to_string(&cfg).unwrap();
        let parsed: ChatConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.me_seed, 1);
        assert_eq!(parsed.bind_port, 4101);
        assert_eq!(parsed.seed_jobs.len(), 1);
        assert!(parsed.chain.is_none());
    }

    #[test]
    fn config_with_chain_serde_round_trip() {
        let json = r#"{
            "enabled": true,
            "me_seed": 1,
            "bind_port": 4101,
            "registry_path": "/tmp/r.json",
            "chain": {
                "rpc_ws": "ws://127.0.0.1:8545",
                "agent_registry": "0x5FbDB2315678afecb367f032d93F642f64180aa3"
            }
        }"#;
        let parsed: ChatConfig = serde_json::from_str(json).unwrap();
        let chain = parsed.chain.expect("chain present");
        assert_eq!(chain.rpc_ws, "ws://127.0.0.1:8545");
        assert!(parsed.seed_jobs.is_empty());
    }

    #[test]
    fn config_default_drive_after_and_seed_jobs_empty() {
        let json = r#"{
            "enabled": false,
            "me_seed": 1,
            "bind_port": 4101,
            "registry_path": "/tmp/r.json"
        }"#;
        let parsed: ChatConfig = serde_json::from_str(json).unwrap();
        assert_eq!(parsed.drive_after_secs, 3);
        assert!(!parsed.drive);
        assert!(parsed.seed_jobs.is_empty());
        assert_eq!(parsed.bootstrappers.len(), 0);
    }

    #[test]
    fn parse_room_key_round_trip() {
        let h = "0102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f20";
        let key = parse_room_key(h).unwrap();
        assert_eq!(key[0], 0x01);
        assert_eq!(key[31], 0x20);
    }

    #[test]
    fn parse_room_key_accepts_0x_prefix() {
        let h = "0x0102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f20";
        let key = parse_room_key(h).unwrap();
        assert_eq!(key[0], 0x01);
        assert_eq!(key[31], 0x20);
    }

    #[test]
    fn parse_room_key_rejects_wrong_length() {
        assert!(matches!(
            parse_room_key("abcd"),
            Err(ChatServiceError::InvalidRoomKey)
        ));
    }

    #[test]
    fn parse_bootstrappers_ok() {
        let pks = parse_bootstrappers(&[
            "1@127.0.0.1:4001".into(),
            "2@10.0.0.5:4002".into(),
        ])
        .unwrap();
        assert_eq!(pks.len(), 2);
        assert_eq!(pks[0].1.port(), 4001);
        assert_eq!(pks[1].1.port(), 4002);
    }

    #[test]
    fn parse_bootstrappers_rejects_malformed() {
        assert!(parse_bootstrappers(&["no_at_sign".into()]).is_err());
        assert!(parse_bootstrappers(&["1@not_an_addr".into()]).is_err());
    }

    #[test]
    fn parse_job_id_u64_ok() {
        assert_eq!(parse_job_id_u64("0").unwrap(), 0);
        assert_eq!(parse_job_id_u64("42").unwrap(), 42);
        assert_eq!(parse_job_id_u64("18446744073709551615").unwrap(), u64::MAX);
    }

    #[test]
    fn parse_job_id_u64_rejects_non_decimal() {
        assert!(matches!(
            parse_job_id_u64("0x42"),
            Err(ChatServiceError::InvalidSeedJobId(_))
        ));
        assert!(matches!(
            parse_job_id_u64(""),
            Err(ChatServiceError::InvalidSeedJobId(_))
        ));
    }

    #[test]
    fn seed_active_jobs_populates_map_with_correct_slot_index() {
        let active: ActiveJobs = Arc::new(Mutex::new(HashMap::new()));
        let seeds = vec![
            SeedJob {
                job_id: "7".into(),
                room_key_hex: "11".repeat(32),
            },
            SeedJob {
                job_id: "42".into(),
                room_key_hex: "22".repeat(32),
            },
        ];
        seed_active_jobs(&active, &seeds).unwrap();

        let map = active.lock().unwrap();
        assert_eq!(map.len(), 2);
        let j7 = map.get(&7).unwrap();
        assert_eq!(j7.job_id, 7);
        assert_eq!(j7.slot_index, room::slot_for_chain_job(7));
        assert_eq!(j7.room_id, room::room_id_for_chain_job(7));
        assert_eq!(j7.room_key, [0x11u8; 32]);
        let j42 = map.get(&42).unwrap();
        assert_eq!(j42.job_id, 42);
        assert_eq!(j42.slot_index, room::slot_for_chain_job(42));
    }

    #[test]
    fn seed_active_jobs_propagates_invalid_id() {
        let active: ActiveJobs = Arc::new(Mutex::new(HashMap::new()));
        let bad = vec![SeedJob {
            job_id: "not_a_number".into(),
            room_key_hex: "00".repeat(32),
        }];
        assert!(matches!(
            seed_active_jobs(&active, &bad),
            Err(ChatServiceError::InvalidSeedJobId(_))
        ));
    }

    #[test]
    fn unwrap_room_key_v1_plaintext_round_trip() {
        let wrap = RoomKeyWrap {
            recipient_pubkey_hex: "0xabc".into(),
            ciphertext_hex: "0x".to_string() + &"77".repeat(32),
        };
        let key = unwrap_room_key(&wrap).unwrap();
        assert_eq!(key, [0x77u8; 32]);
    }
}
