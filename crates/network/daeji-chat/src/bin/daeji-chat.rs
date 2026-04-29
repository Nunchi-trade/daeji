//! Phase-2 POC for the Daeji × commonware-chat agent-coordination spec.
//!
//! - Authorized peer set comes from a registry file (stand-in for on-chain `IdentityRegistry`).
//! - A background task polls the registry every `REGISTRY_POLL_INTERVAL` and calls
//!   `oracle.track(new_epoch, new_set)` whenever the registry's epoch changes.
//! - This is the structural feature from spec D2: chain block height (here: registry epoch)
//!   drives the commonware peer-set oracle.
//!
//! Spec: ~/obsidian-vault/projects/2026-04-29-daeji-commonware-chat-spec.md

use clap::Parser;
use commonware_cryptography::{ed25519, Signer as _};
use commonware_p2p::{
    authenticated::discovery, Manager as _, Receiver, Recipients, Sender,
};
use commonware_runtime::{tokio, Clock, Metrics, Quota, Runner as _, Spawner};
use daeji_chat::{messages::RoomMessage, registry::Registry, room};
use std::{
    net::{IpAddr, Ipv4Addr, SocketAddr},
    path::PathBuf,
    str::FromStr,
    time::Duration,
};
use tracing::{debug, info, warn};

const APPLICATION_NAMESPACE: &[u8] = b"_DAEJI_CHAT_POC_V1";
const MAX_MESSAGE_SIZE: u32 = 8 * 1024;
const MAX_BACKLOG: usize = 256;
const REGISTRY_POLL_INTERVAL: Duration = Duration::from_millis(200);

#[derive(Parser, Debug)]
#[command(version, about)]
struct Args {
    /// Identity in `<seed>@<port>` form. Seed is hashed deterministically into an
    /// ed25519 key (matches the upstream chat example's convention).
    #[arg(long)]
    me: String,

    /// Registry file path. Polled every 200ms; epoch bumps trigger oracle.track.
    /// In real Phase 2 this is replaced by an `eth_subscribe` against IdentityRegistry.
    #[arg(long)]
    registry_path: PathBuf,

    /// Bootstrappers in `<seed>@<host:port>` form, comma-separated.
    /// Stays a CLI flag in this POC because commonware-p2p locks bootstrappers at config time.
    #[arg(long, value_delimiter = ',', default_values_t = Vec::<String>::new())]
    bootstrappers: Vec<String>,

    /// Job id used to derive the room (any utf-8 string the participants agree on).
    #[arg(long, default_value = "demo-job-1")]
    job_id: String,

    /// 32-byte hex room key, pre-shared by all members for this POC.
    /// Phase 2.5+ replaces with the X25519 ECDH handshake on JobAwarded.
    #[arg(long)]
    room_key_hex: String,

    /// If set, this process drives the demo: sends Hello, then a Status, then a Final
    /// after a fixed startup delay so peers have time to connect.
    #[arg(long, default_value_t = false)]
    drive: bool,

    /// Seconds after startup at which the drive sequence fires (only with --drive).
    #[arg(long, default_value_t = 3)]
    drive_after_secs: u64,
}

fn parse_me(me: &str) -> (u64, u16) {
    let (seed, port) = me.split_once('@').expect("--me must be <seed>@<port>");
    (
        seed.parse().expect("seed not a u64"),
        port.parse().expect("port not a u16"),
    )
}

fn parse_bootstrapper(s: &str) -> (u64, SocketAddr) {
    let (seed, addr) = s
        .split_once('@')
        .expect("bootstrapper must be <seed>@<addr>");
    (
        seed.parse().expect("bootstrapper seed not a u64"),
        SocketAddr::from_str(addr).expect("bootstrapper addr malformed"),
    )
}

fn parse_room_key(hex_str: &str) -> [u8; 32] {
    let raw = hex::decode(hex_str.trim()).expect("--room-key-hex not valid hex");
    assert_eq!(raw.len(), 32, "--room-key-hex must be 32 bytes (64 hex chars)");
    let mut out = [0u8; 32];
    out.copy_from_slice(&raw);
    out
}

fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let args = Args::parse();
    let (my_seed, my_port) = parse_me(&args.me);
    let signer = ed25519::PrivateKey::from_seed(my_seed);
    info!(seed = my_seed, port = my_port, key = ?signer.public_key(), "loaded signer");

    let bootstrappers: Vec<_> = args
        .bootstrappers
        .iter()
        .map(|s| {
            let (seed, addr) = parse_bootstrapper(s);
            (
                ed25519::PrivateKey::from_seed(seed).public_key(),
                addr.into(),
            )
        })
        .collect();

    let p2p_cfg = discovery::Config::local(
        signer.clone(),
        APPLICATION_NAMESPACE,
        SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), my_port),
        SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), my_port),
        bootstrappers,
        MAX_MESSAGE_SIZE,
    );

    let room = room::room_id(args.job_id.as_bytes());
    let channel_id = room::channel_id_from_room(&room);
    let room_key = parse_room_key(&args.room_key_hex);
    let registry_path = args.registry_path.clone();
    info!(
        job_id = %args.job_id,
        room_hex = %hex::encode(room),
        channel_id,
        registry = %registry_path.display(),
        "room configured"
    );

    let executor = tokio::Runner::default();
    executor.start(|context| async move {
        let (mut network, mut oracle) =
            discovery::Network::new(context.with_label("network"), p2p_cfg);

        // Initial peer-set load from the registry. Real Phase 2 awaits the indexer's
        // first snapshot; the POC just reads the file once.
        let initial = match Registry::load(&registry_path) {
            Ok(r) => r,
            Err(err) => {
                warn!(?err, registry = %registry_path.display(), "registry load failed; starting empty");
                Registry::empty()
            }
        };
        info!(epoch = initial.epoch, agents = ?initial.seeds(), "initial peer-set");
        oracle.track(initial.epoch, initial.pubkey_set()).await;

        let (sender, receiver) =
            network.register(channel_id, Quota::per_second(NZU32!(64)), MAX_BACKLOG);

        // Background task: poll the registry and fire oracle.track on epoch change.
        // Exploits Spawner: Clone (and therefore Self: Clone) plus the concrete tokio
        // Context implementing Clock — we capture a cloned context that retains Clock.
        let watcher_ctx = context.clone();
        let mut watcher_oracle = oracle.clone();
        let watcher_path = registry_path.clone();
        let mut last_epoch = initial.epoch;
        context.with_label("registry-watcher").spawn(move |_| async move {
            loop {
                watcher_ctx.sleep(REGISTRY_POLL_INTERVAL).await;
                match Registry::load(&watcher_path) {
                    Ok(reg) if reg.epoch != last_epoch => {
                        info!(
                            old_epoch = last_epoch,
                            new_epoch = reg.epoch,
                            agents = ?reg.seeds(),
                            "registry epoch advanced — refreshing oracle peer-set"
                        );
                        watcher_oracle.track(reg.epoch, reg.pubkey_set()).await;
                        last_epoch = reg.epoch;
                    }
                    Ok(_) => {}
                    Err(err) => {
                        debug!(?err, "registry poll error (transient?)");
                    }
                }
            }
        });

        // Optional driver task: fires the symphony sequence after a fixed delay so peers
        // have time to discover one another via the registry-driven oracle.track.
        if args.drive {
            let driver_ctx = context.clone();
            let mut driver_sender = sender.clone();
            let me_hex = hex::encode(signer.public_key().as_ref());
            let drive_delay = Duration::from_secs(args.drive_after_secs);
            context.with_label("driver").spawn(move |_| async move {
                driver_ctx.sleep(drive_delay).await;
                drive_sequence(&mut driver_sender, &room_key, &room, &me_hex).await;
            });
        }

        let network_handler = network.start();

        run_room(sender, receiver, signer, room, room_key).await;

        network_handler.abort();
    });
}

async fn run_room<S, R>(
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
    info!(me = %short(&me_hex), "entering room loop");

    loop {
        match receiver.recv().await {
            Ok((peer, wire)) => {
                let peer_hex = hex::encode(peer.as_ref());
                let plaintext = match room::decrypt(&room_key, &room, wire.as_ref()) {
                    Some(pt) => pt,
                    None => {
                        debug!(peer = %short(&peer_hex), "ciphertext failed AEAD — wrong key or not for me");
                        continue;
                    }
                };
                let msg: RoomMessage = match serde_json::from_slice(&plaintext) {
                    Ok(m) => m,
                    Err(err) => {
                        warn!(peer = %short(&peer_hex), ?err, "malformed room message after decrypt");
                        continue;
                    }
                };
                info!(
                    peer = %short(&peer_hex),
                    kind = msg.label(),
                    "rx {}",
                    pretty(&msg),
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
                debug!(?err, "receiver error");
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
    info!(me = %short(me_hex), "driver starting");
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

    // Small spacing between sends so peers can interleave; commonware backlog handles it.
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

/// Tiny inter-send pause without needing Clock — yields ~`hops` times to the runtime.
/// Sufficient for receiver ordering in the POC; replaced by Clock-driven sleep elsewhere.
async fn spin_yield(hops: usize) {
    for _ in 0..hops {
        futures::future::ready(()).await;
        // tokio's yield_now would be cleaner but we don't depend on raw tokio here.
    }
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
            info!(kind = msg.label(), reached = reached.len(), "tx");
        }
        Ok(_) => {
            warn!(kind = msg.label(), "tx — no peers reached (offline?)");
        }
        Err(err) => {
            warn!(kind = msg.label(), ?err, "tx failed");
        }
    }
}

fn short(hex_pk: &str) -> String {
    if hex_pk.len() < 8 {
        return hex_pk.to_string();
    }
    format!("{}..{}", &hex_pk[..4], &hex_pk[hex_pk.len() - 4..])
}

fn pretty(msg: &RoomMessage) -> String {
    serde_json::to_string(msg).unwrap_or_else(|_| msg.label().into())
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

// Bring the `NZU32!` macro into scope for the `Quota::per_second` call above.
use commonware_utils::NZU32;
