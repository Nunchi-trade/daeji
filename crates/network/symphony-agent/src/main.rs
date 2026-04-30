//! `symphony-agent` — minimal multi-agent chat agent that demonstrates
//! "agents talking in a chat" using the **same commonware-p2p library** as
//! the kora chat layer.
//!
//! This is the answer to: "we want agents talking in a chat" + "we need
//! agents actually doing this." Each instance is a real Rust process that:
//!
//! 1. Sets up a commonware-p2p discovery network (same primitives as kora's
//!    chat layer in `daeji-chat::service::run_chat`)
//! 2. Tracks the peer set via the oracle (deterministic — peer pubkeys are
//!    derived from CLI seeds)
//! 3. Registers a shared "symphony-demo" channel (room id derived from a
//!    job id via `keccak256("DAEJI_ROOM_V1" || job_id)`, matching the
//!    on-chain `MultiAgentMarket.computeRoomId`)
//! 4. AEAD-encrypts outgoing messages with a shared room key (ChaCha20Poly1305,
//!    room id bound as AAD — same as kora's chat layer)
//! 5. Sends a periodic monologue: Hello → Status → PartialResult → Vote → Final
//! 6. Receives + decrypts other agents' messages, logs them with sender +
//!    message kind
//!
//! Three agents on localhost demonstrate cross-process AEAD chat.
//!
//! What this is NOT (yet):
//! - Not coordinated through the lobby (no JobAnnounce; the room key is
//!   shared via CLI rather than ECDH-wrapped per recipient — full lobby +
//!   wrap is in `daeji-chat::lobby` and lands when symphony-agent integrates
//!   with on-chain MultiAgentMarket events)
//! - Not yet wired to chain settlement (no `submitMulti` co-signing; that's
//!   the next layer once cross-talk is verified)

#![allow(missing_docs)]

use std::{
    net::{IpAddr, Ipv4Addr, SocketAddr},
    str::FromStr,
    time::Duration,
};

use clap::Parser;
use commonware_cryptography::{ed25519, Signer as _};
use commonware_p2p::{
    authenticated::discovery::{self, Config as DiscoveryConfig},
    Manager as _, Recipients,
};
use commonware_runtime::{tokio as cwtokio, Clock, Metrics, Quota, Runner, Spawner};
use commonware_utils::{ordered::Set, NZU32};
use daeji_chat::{messages::RoomMessage, room};
use eyre::{Context, Result};
use tracing::{debug, info, warn};

/// Application namespace shared with `daeji-chat::service::APPLICATION_NAMESPACE`
/// so future symphony-agent ↔ kora-chat-layer talk works without a
/// namespace mismatch.
const APPLICATION_NAMESPACE: &[u8] = b"_DAEJI_CHAT_V1";

const MAX_MESSAGE_SIZE: u32 = 8 * 1024;
const MAX_BACKLOG: usize = 256;

#[derive(Debug, Parser)]
#[command(
    name = "symphony-agent",
    version,
    about = "minimal multi-agent chat agent demonstrating 'agents talking in a chat' via commonware-p2p"
)]
struct Cli {
    /// Human-readable agent name for log output (e.g. "alice", "bob").
    #[arg(long, env = "AGENT_NAME")]
    name: String,

    /// ed25519 seed for this agent's transport identity (1-based).
    #[arg(long, env = "AGENT_SEED")]
    seed: u64,

    /// Local TCP port to bind for the chat mesh.
    #[arg(long, env = "AGENT_PORT")]
    port: u16,

    /// Comma-separated list of all peer seeds (e.g. "1,2,3"). The agent's
    /// own seed must be included. Used to derive the deterministic peer
    /// public-key set for `oracle.track`.
    #[arg(long, env = "AGENT_PEER_SEEDS", value_delimiter = ',')]
    peer_seeds: Vec<u64>,

    /// Comma-separated bootstrappers as `<seed>@<host:port>`. Provide at
    /// least one peer here so the discovery handshake can begin. The agent's
    /// own entry should NOT be included.
    #[arg(long, env = "AGENT_BOOTSTRAPPERS", value_delimiter = ',')]
    bootstrappers: Vec<String>,

    /// Job id for room derivation. All agents in the same demo must use
    /// the same value. Default 42 matches the §21 demo conventions.
    #[arg(long, env = "AGENT_JOB_ID", default_value = "42")]
    job_id: u64,

    /// 32-byte hex room key (pre-shared in v1; ECDH-wrap lands in PR-Daeji-F).
    /// All agents in the same demo must use the same value. Default = 32 zero
    /// bytes (insecure but fine for the localhost demo).
    #[arg(
        long,
        env = "AGENT_ROOM_KEY",
        default_value = "0000000000000000000000000000000000000000000000000000000000000000"
    )]
    room_key_hex: String,

    /// Seconds between outgoing messages.
    #[arg(long, env = "AGENT_INTERVAL_SECS", default_value = "2")]
    interval_secs: u64,

    /// Stop after N self-sent messages (0 = forever).
    #[arg(long, env = "AGENT_MAX_TICKS", default_value = "10")]
    max_ticks: u32,
}

fn main() -> Result<()> {
    let _ = tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info,symphony_agent=info".into()),
        )
        .with_target(false)
        .try_init();

    let cli = Cli::parse();
    let name = cli.name.clone();

    info!(
        agent = %name,
        seed = cli.seed,
        port = cli.port,
        peer_seeds = ?cli.peer_seeds,
        job_id = cli.job_id,
        "symphony-agent starting"
    );

    let executor = cwtokio::Runner::default();
    executor.start(|context| async move {
        if let Err(err) = run(context, cli).await {
            tracing::error!(?err, "symphony-agent failed");
        }
    });

    Ok(())
}

async fn run<C>(context: C, cli: Cli) -> Result<()>
where
    C: Spawner + Metrics + Clock + commonware_runtime::BufferPooler + commonware_runtime::Network + commonware_runtime::Resolver + rand_core::CryptoRngCore,
{
    let signer = ed25519::PrivateKey::from_seed(cli.seed);
    let me_pubkey_hex = format!("0x{}", hex::encode(signer.public_key().as_ref()));

    info!(agent = %cli.name, pubkey = %me_pubkey_hex, "loaded signer");

    // Parse bootstrappers: "<seed>@<host:port>".
    let bootstrappers = parse_bootstrappers(&cli.bootstrappers)?;

    let bind = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), cli.port);
    let p2p_cfg = DiscoveryConfig::local(
        signer.clone(),
        APPLICATION_NAMESPACE,
        bind,
        bind, // advertise = bind (loopback demo)
        bootstrappers,
        MAX_MESSAGE_SIZE,
    );

    let (mut network, mut oracle) =
        discovery::Network::new(context.with_label("symphony-network"), p2p_cfg);

    // Build deterministic peer set from the seed list. All agents do this
    // identically so discovery converges.
    let peers: Set<ed25519::PublicKey> = Set::from_iter_dedup(
        cli.peer_seeds
            .iter()
            .map(|&s| ed25519::PrivateKey::from_seed(s).public_key()),
    );
    info!(agent = %cli.name, peer_count = cli.peer_seeds.len(), "tracking peer set");
    oracle.track(0, peers).await;

    // Register the shared symphony-demo channel. Use the on-chain-equivalent
    // room id derivation — `room::room_id_for_chain_job(job_id)` matches the
    // Solidity `MultiAgentMarket.computeRoomId(jobId)` byte-for-byte.
    let room_id = room::room_id_for_chain_job(cli.job_id);
    let channel_id = room::channel_id_from_room(&room_id);
    let room_key = parse_room_key(&cli.room_key_hex)?;
    info!(
        agent = %cli.name,
        job_id = cli.job_id,
        room_id_hex = %hex::encode(room_id),
        channel_id,
        "registering symphony-demo channel"
    );
    let (sender, receiver) =
        network.register(channel_id, Quota::per_second(NZU32!(64)), MAX_BACKLOG);

    // Spawn the receiver task: prints incoming AEAD-decrypted messages.
    let recv_name = cli.name.clone();
    context
        .with_label("symphony-recv")
        .spawn(move |_| async move {
            run_recv_loop(receiver, room_key, room_id, recv_name).await;
        });

    // Optional grace period before sending — gives all peers time to discover
    // each other so the first messages don't get dropped.
    let network_handler = network.start();
    info!(agent = %cli.name, "network started; sleeping 3s for peer discovery");
    tokio::time::sleep(Duration::from_secs(3)).await;

    // Send loop: cycle through Hello → Status → PartialResult → Vote → Final.
    let mut tick: u32 = 0;
    let mut sender_ref = sender;
    loop {
        tick += 1;
        let msg = compose_message(&cli.name, &me_pubkey_hex, tick);
        let kind = msg.label().to_string();
        send_room_message(&mut sender_ref, &room_key, &room_id, &msg).await;
        info!(agent = %cli.name, tick, kind, "sent");

        if cli.max_ticks > 0 && tick >= cli.max_ticks {
            info!(agent = %cli.name, tick, "max_ticks reached; sleeping briefly to flush + exit");
            tokio::time::sleep(Duration::from_secs(2)).await;
            break;
        }
        tokio::time::sleep(Duration::from_secs(cli.interval_secs)).await;
    }

    let _ = network_handler.await;
    Ok(())
}

/// Compose a deterministic-looking message based on tick. Cycles through
/// the symphony message types so the demo shows ALL message kinds in flight.
fn compose_message(name: &str, me_pubkey_hex: &str, tick: u32) -> RoomMessage {
    let from = me_pubkey_hex.to_string();
    match tick % 5 {
        1 => RoomMessage::Hello {
            from_pubkey_hex: from,
            wall_clock_ms: now_ms(),
        },
        2 => RoomMessage::Status {
            from_pubkey_hex: from,
            phase: format!("{name} executing"),
            eta_blocks: 5,
        },
        3 => RoomMessage::PartialResult {
            from_pubkey_hex: from,
            partial_id: tick as u64,
            content_hash_hex: format!("0x{}", "ab".repeat(32)),
            content_ref: format!("{name}/partial/{tick}"),
        },
        4 => RoomMessage::Vote {
            from_pubkey_hex: from,
            partial_id: tick as u64,
            agree: true,
        },
        _ => RoomMessage::Final {
            from_pubkey_hex: from,
            result_hash_hex: format!("0x{}", "cd".repeat(32)),
            signature_hex: format!("0x{}", "ef".repeat(65)),
        },
    }
}

async fn send_room_message<S: commonware_p2p::Sender>(
    sender: &mut S,
    room_key: &[u8; 32],
    room_id: &[u8; 32],
    msg: &RoomMessage,
) {
    let plaintext = serde_json::to_vec(msg).expect("serialize");
    let wire = room::encrypt(room_key, room_id, &plaintext);
    match sender.send(Recipients::All, wire, false).await {
        Ok(reached) if !reached.is_empty() => {
            debug!(reached = reached.len(), "tx fanout");
        }
        Ok(_) => warn!(kind = msg.label(), "tx — no peers reached"),
        Err(err) => warn!(kind = msg.label(), ?err, "tx failed"),
    }
}

async fn run_recv_loop<R: commonware_p2p::Receiver>(
    mut receiver: R,
    room_key: [u8; 32],
    room_id: [u8; 32],
    me_name: String,
) {
    info!(agent = %me_name, "recv loop started");
    loop {
        match receiver.recv().await {
            Ok((peer, wire)) => {
                let peer_hex = hex::encode(peer.as_ref());
                let plaintext = match room::decrypt(&room_key, &room_id, wire.as_ref()) {
                    Some(pt) => pt,
                    None => {
                        debug!(peer = %short(&peer_hex), "AEAD decrypt failed (wrong key / not for me)");
                        continue;
                    }
                };
                let msg: RoomMessage = match serde_json::from_slice(&plaintext) {
                    Ok(m) => m,
                    Err(err) => {
                        warn!(?err, "malformed RoomMessage after decrypt");
                        continue;
                    }
                };
                let preview = format_preview(&msg);
                info!(
                    agent = %me_name,
                    from_peer = %short(&peer_hex),
                    kind = msg.label(),
                    preview = %preview,
                    "RX"
                );
            }
            Err(err) => {
                debug!(?err, "recv error");
            }
        }
    }
}

fn format_preview(msg: &RoomMessage) -> String {
    match msg {
        RoomMessage::Hello { wall_clock_ms, .. } => format!("hello @ {wall_clock_ms}"),
        RoomMessage::Status { phase, eta_blocks, .. } => format!("\"{phase}\" eta={eta_blocks}"),
        RoomMessage::PartialResult { partial_id, content_ref, .. } => {
            format!("partial #{partial_id} {content_ref}")
        }
        RoomMessage::Vote { partial_id, agree, .. } => format!("vote on #{partial_id} = {agree}"),
        RoomMessage::Final { result_hash_hex, .. } => {
            format!("final hash={}…", &result_hash_hex[..10])
        }
    }
}

fn parse_bootstrappers(
    raw: &[String],
) -> Result<Vec<(ed25519::PublicKey, commonware_p2p::Ingress)>> {
    raw.iter()
        .map(|s| {
            let (seed_str, addr_str) = s
                .split_once('@')
                .ok_or_else(|| eyre::eyre!("bootstrapper missing '@': {s}"))?;
            let seed: u64 = seed_str.parse().with_context(|| format!("bad seed: {seed_str}"))?;
            let addr = SocketAddr::from_str(addr_str)
                .with_context(|| format!("bad addr: {addr_str}"))?;
            Ok((ed25519::PrivateKey::from_seed(seed).public_key(), addr.into()))
        })
        .collect()
}

fn parse_room_key(s: &str) -> Result<[u8; 32]> {
    let trimmed = s.trim().strip_prefix("0x").unwrap_or(s.trim());
    let raw = hex::decode(trimmed).context("room key hex decode")?;
    if raw.len() != 32 {
        eyre::bail!("room key must be 32 bytes (got {})", raw.len());
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
