//! Chat service entry point — bolted into the kora chain binary.
//!
//! `run_chat(ctx, config)` is the function kora calls when `--enable-chat` is set.
//! It builds a commonware-p2p `discovery::Network` on a configurable port (separate
//! from kora's consensus network for v1 — see canonical-plan §13 for the rationale),
//! tracks the authorized peer set from a registry file, registers a room channel,
//! and runs the symphony message loop.
//!
//! The chain-event subscription (`AgentRegistry.AgentRegistered`,
//! `MultiAgentMarket.JobAwarded`) is a follow-up PR; v1 reads the registry from
//! a JSON file the operator maintains (or that an external watcher updates).

use std::{
    net::{IpAddr, Ipv4Addr, SocketAddr},
    path::PathBuf,
    str::FromStr,
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

    /// Job id used to derive the room (any utf-8 string the participants agree on).
    pub job_id: String,

    /// 32-byte hex room key (pre-shared in v1; replaced by ECDH in PR-Daeji-C).
    pub room_key_hex: String,

    /// Drive a demo sequence (Hello → Status → Final) `drive_after_secs` after start.
    #[serde(default)]
    pub drive: bool,

    /// Seconds after startup at which the drive sequence fires (only with `drive`).
    #[serde(default = "default_drive_after")]
    pub drive_after_secs: u64,
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
    #[error("registry load failed: {0}")]
    Registry(#[from] std::io::Error),
}

/// Run the chat service. Builds a commonware-p2p network on the configured port,
/// tracks the registry-driven peer set, registers the room channel, and runs the
/// message loop. Returns when the network shuts down.
///
/// kora calls this from `LegacyNodeService.run_with_context` (or equivalent) as
/// a spawned tokio task when `--enable-chat` is set:
///
/// ```ignore
/// if config.chat.as_ref().map_or(false, |c| c.enabled) {
///     let chat_ctx = context.clone();
///     let chat_cfg = config.chat.clone().unwrap();
///     context.with_label("chat").spawn(move |_| async move {
///         if let Err(err) = daeji_chat::service::run_chat(chat_ctx, chat_cfg).await {
///             tracing::error!(?err, "chat service failed");
///         }
///     });
/// }
/// ```
pub async fn run_chat<C>(context: C, config: ChatConfig) -> Result<(), ChatServiceError>
where
    C: Spawner + Metrics + Clock + commonware_runtime::BufferPooler + commonware_runtime::Network + commonware_runtime::Resolver + rand_core::CryptoRngCore,
{
    if !config.enabled {
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

    let p2p_cfg = DiscoveryConfig::local(
        signer.clone(),
        APPLICATION_NAMESPACE,
        SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), config.bind_port),
        SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), config.bind_port),
        bootstrappers,
        MAX_MESSAGE_SIZE,
    );

    let room_id = room::room_id(config.job_id.as_bytes());
    let channel_id = room::channel_id_from_room(&room_id);
    let room_key = parse_room_key(&config.room_key_hex)?;
    info!(
        job_id = %config.job_id,
        room_hex = %hex::encode(room_id),
        channel_id,
        "chat: room configured"
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

    let (sender, receiver) =
        network.register(channel_id, Quota::per_second(NZU32!(64)), MAX_BACKLOG);

    // Background task: poll the registry file and refresh oracle.track on epoch change.
    let watcher_ctx = context.clone();
    let mut watcher_oracle = oracle.clone();
    let watcher_path = config.registry_path.clone();
    let mut last_epoch = initial.epoch;
    context.with_label("chat-registry-watcher").spawn(move |_| async move {
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

    // Optional drive sequence — fires once after a fixed delay so peers have time to discover.
    if config.drive {
        let driver_ctx = context.clone();
        let mut driver_sender = sender.clone();
        let me_hex = hex::encode(signer.public_key().as_ref());
        let drive_delay = Duration::from_secs(config.drive_after_secs);
        let driver_room = room_id;
        let driver_key = room_key;
        context.with_label("chat-driver").spawn(move |_| async move {
            driver_ctx.sleep(drive_delay).await;
            drive_sequence(&mut driver_sender, &driver_key, &driver_room, &me_hex).await;
        });
    }

    let network_handler = network.start();

    // Run the room message loop until the network shuts down.
    run_room_loop(sender, receiver, signer, room_id, room_key).await;

    network_handler.abort();
    Ok(())
}

async fn run_room_loop<S, R>(
    mut sender: S,
    mut receiver: R,
    signer: ed25519::PrivateKey,
    room: [u8; 32],
    room_key: [u8; 32],
) where
    S: Sender,
    R: Receiver,
{
    let me_hex = hex::encode(signer.public_key().as_ref());
    info!(me = %short(&me_hex), "chat: entering room loop");

    loop {
        match receiver.recv().await {
            Ok((peer, wire)) => {
                let peer_hex = hex::encode(peer.as_ref());
                let plaintext = match room::decrypt(&room_key, &room, wire.as_ref()) {
                    Some(pt) => pt,
                    None => {
                        debug!(peer = %short(&peer_hex), "chat: ciphertext failed AEAD — wrong key or not for me");
                        continue;
                    }
                };
                let msg: RoomMessage = match serde_json::from_slice(&plaintext) {
                    Ok(m) => m,
                    Err(err) => {
                        warn!(peer = %short(&peer_hex), ?err, "chat: malformed room message after decrypt");
                        continue;
                    }
                };
                info!(
                    peer = %short(&peer_hex),
                    kind = msg.label(),
                    "chat: rx"
                );

                if let RoomMessage::Final { .. } = &msg {
                    let reply = RoomMessage::Vote {
                        from_pubkey_hex: me_hex.clone(),
                        partial_id: 0,
                        agree: true,
                    };
                    send_room(&mut sender, &room_key, &room, &reply).await;
                }
            }
            Err(err) => {
                debug!(?err, "chat: receiver error");
            }
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
    let raw = hex::decode(hex_str.trim()).map_err(|_| ChatServiceError::InvalidRoomKey)?;
    if raw.len() != 32 {
        return Err(ChatServiceError::InvalidRoomKey);
    }
    let mut out = [0u8; 32];
    out.copy_from_slice(&raw);
    Ok(out)
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
            job_id: "demo".into(),
            room_key_hex: "00".repeat(32),
            drive: false,
            drive_after_secs: 3,
        };
        let json = serde_json::to_string(&cfg).unwrap();
        let parsed: ChatConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.me_seed, 1);
        assert_eq!(parsed.bind_port, 4101);
    }

    #[test]
    fn config_default_drive_after() {
        let json = r#"{
            "enabled": false,
            "me_seed": 1,
            "bind_port": 4101,
            "registry_path": "/tmp/r.json",
            "job_id": "j",
            "room_key_hex": "0000000000000000000000000000000000000000000000000000000000000000"
        }"#;
        let parsed: ChatConfig = serde_json::from_str(json).unwrap();
        assert_eq!(parsed.drive_after_secs, 3);
        assert_eq!(parsed.drive, false);
        assert_eq!(parsed.bootstrappers.len(), 0);
    }

    #[test]
    fn parse_room_key_round_trip() {
        let hex = "0102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f20";
        let key = parse_room_key(hex).unwrap();
        assert_eq!(key[0], 0x01);
        assert_eq!(key[31], 0x20);
    }

    #[test]
    fn parse_room_key_rejects_wrong_length() {
        assert!(matches!(parse_room_key("abcd"), Err(ChatServiceError::InvalidRoomKey)));
    }

    #[test]
    fn parse_bootstrappers_ok() {
        let pks = parse_bootstrappers(&["1@127.0.0.1:4001".into(), "2@10.0.0.5:4002".into()]).unwrap();
        assert_eq!(pks.len(), 2);
        assert_eq!(pks[0].1.port(), 4001);
        assert_eq!(pks[1].1.port(), 4002);
    }

    #[test]
    fn parse_bootstrappers_rejects_malformed() {
        assert!(parse_bootstrappers(&["no_at_sign".into()]).is_err());
        assert!(parse_bootstrappers(&["1@not_an_addr".into()]).is_err());
    }
}
