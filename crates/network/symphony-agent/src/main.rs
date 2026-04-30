//! `symphony-agent` — multi-agent Rust process that demonstrates the FULL
//! canonical-plan §6 flow with real chat coordination + on-chain settlement.
//!
//! What it does end-to-end:
//!
//! 1. Connects to chain via alloy WS (the SAME as `isfr-keeper-agent`)
//! 2. Connects to the chat mesh via commonware-p2p (the SAME as kora's
//!    chat layer)
//! 3. Subscribes to `MultiAgentMarket.JobAwarded` events
//! 4. When awarded a job:
//!    a. Joins the chat room (room id = `keccak256("DAEJI_ROOM_V1" || jobId)`)
//!    b. Broadcasts `Hello` + `Status` + `PartialResult`
//!    c. Computes a deterministic result hash agreed by all agents
//!    d. Signs `keccak256("DAEJI_RESULT_V1" || id || resultHash)` with its
//!       EVM private key
//!    e. Broadcasts `Final{ result_hash, signature }`
//!    f. Receives the other winners' `Final` messages
//!    g. The lexicographically-lowest winner address calls
//!       `submitMulti(id, resultHash, [sig1, sig2, ...])`
//! 5. The on-chain resolver (deployer in the demo) accepts → bounty paid
//!    + reputation updated
//!
//! This closes step ⑥ of canonical-plan §6 — on-chain settlement following
//! chat coordination.

#![allow(missing_docs)]

use std::{
    collections::HashMap,
    net::{IpAddr, Ipv4Addr, SocketAddr},
    str::FromStr,
    sync::{Arc, Mutex},
    time::Duration,
};

use alloy::{
    network::EthereumWallet,
    primitives::{keccak256, Address, Bytes as AlloyBytes, FixedBytes, Log as AlloyLog, U256},
    providers::{Provider, ProviderBuilder, WsConnect},
    rpc::types::eth::{Filter, Log as AlloyRpcLog},
    signers::{local::PrivateKeySigner, Signer},
    sol,
    sol_types::SolEvent,
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
use futures::StreamExt;
use tracing::{debug, error, info, warn};

const APPLICATION_NAMESPACE: &[u8] = b"_DAEJI_CHAT_V1";
const MAX_MESSAGE_SIZE: u32 = 8 * 1024;
const MAX_BACKLOG: usize = 256;
const RESULT_DOMAIN: &[u8] = b"DAEJI_RESULT_V1";

sol! {
    #[derive(Debug)]
    event JobAwarded(uint256 indexed id, address[] winners, bytes32 roomId);

    #[sol(rpc)]
    interface IMultiAgentMarket {
        function submitMulti(uint256 id, bytes32 resultHash, bytes[] calldata signatures) external;
        function getWinners(uint256 id) external view returns (address[] memory);
        function resultDigest(uint256 id, bytes32 resultHash) external pure returns (bytes32);
    }
}

#[derive(Debug, Parser)]
#[command(
    name = "symphony-agent",
    version,
    about = "multi-agent Rust process: chat coordination + on-chain MultiAgentMarket settlement"
)]
struct Cli {
    /// Human-readable agent name for log output (e.g. "alice", "bob").
    #[arg(long, env = "AGENT_NAME")]
    name: String,

    /// ed25519 seed for chat transport identity (1-based).
    #[arg(long, env = "AGENT_SEED")]
    seed: u64,

    /// Local TCP port for the chat mesh.
    #[arg(long, env = "AGENT_PORT")]
    port: u16,

    /// Comma-separated list of all peer seeds.
    #[arg(long, env = "AGENT_PEER_SEEDS", value_delimiter = ',')]
    peer_seeds: Vec<u64>,

    /// Chat bootstrappers: `<seed>@<host:port>` comma-separated.
    #[arg(long, env = "AGENT_BOOTSTRAPPERS", value_delimiter = ',')]
    bootstrappers: Vec<String>,

    /// 32-byte hex room key (pre-shared in v1).
    #[arg(
        long,
        env = "AGENT_ROOM_KEY",
        default_value = "0000000000000000000000000000000000000000000000000000000000000000"
    )]
    room_key_hex: String,

    /// EVM private key (0x-hex). Used for both the on-chain submitMulti tx
    /// AND for signing the result digest. Defaults to anvil keys when seed is 1-5.
    #[arg(long, env = "AGENT_ETH_PRIVKEY")]
    eth_privkey: String,

    /// Chain RPC WebSocket endpoint.
    #[arg(long, env = "AGENT_RPC_WS")]
    rpc_ws: String,

    /// MultiAgentMarket contract address (0x-hex).
    #[arg(long, env = "AGENT_MULTI_AGENT_MARKET")]
    multi_agent_market: String,

    /// How many seconds to coordinate over chat before signing + broadcasting
    /// the Final message.
    #[arg(long, env = "AGENT_CHAT_GRACE_SECS", default_value = "4")]
    chat_grace_secs: u64,

    /// Stop the agent after one job is settled. Useful for the demo so the
    /// process exits cleanly. 0 = run forever.
    #[arg(long, env = "AGENT_EXIT_AFTER_SETTLE", default_value = "true")]
    exit_after_settle: bool,
}

/// State machine entry: a job we were awarded and are coordinating on.
#[derive(Clone)]
struct ActiveJob {
    job_id: u64,
    winners: Vec<Address>,
    room_id: [u8; 32],
    /// Sigs collected from the room (keyed by winner address).
    collected_sigs: HashMap<Address, Vec<u8>>,
    /// The deterministic result hash all winners agree on.
    result_hash: [u8; 32],
    /// Whether we've sent our Final message already (don't send twice).
    final_sent: bool,
    /// Whether we've called submitMulti already (don't submit twice).
    submitted: bool,
}

type ActiveJobs = Arc<Mutex<HashMap<u64, ActiveJob>>>;

fn main() -> Result<()> {
    let _ = tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info,symphony_agent=info".into()),
        )
        .with_target(false)
        .try_init();

    let cli = Cli::parse();
    info!(
        agent = %cli.name,
        seed = cli.seed,
        port = cli.port,
        rpc_ws = %cli.rpc_ws,
        multi_agent_market = %cli.multi_agent_market,
        "symphony-agent (chain + chat) starting"
    );

    let executor = cwtokio::Runner::default();
    executor.start(|context| async move {
        if let Err(err) = run(context, cli).await {
            tracing::error!(?err, "symphony-agent failed");
            std::process::exit(1);
        }
    });

    Ok(())
}

async fn run<C>(context: C, cli: Cli) -> Result<()>
where
    C: Spawner
        + Metrics
        + Clock
        + commonware_runtime::BufferPooler
        + commonware_runtime::Network
        + commonware_runtime::Resolver
        + rand_core::CryptoRngCore,
{
    // ---- Identities ----
    let signer = ed25519::PrivateKey::from_seed(cli.seed);
    let me_pubkey_hex = format!("0x{}", hex::encode(signer.public_key().as_ref()));

    let eth_signer: PrivateKeySigner = cli
        .eth_privkey
        .parse()
        .context("invalid eth privkey (expect 0x-hex)")?;
    let me_eth_addr = eth_signer.address();
    info!(agent = %cli.name, eth_addr = %me_eth_addr, "loaded eth signer");

    let market_addr: Address = cli.multi_agent_market.parse().context("market addr")?;

    // ---- Chat network setup (same as the standalone-chat version) ----
    let bootstrappers = parse_bootstrappers(&cli.bootstrappers)?;
    let bind = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), cli.port);
    let p2p_cfg = DiscoveryConfig::local(
        signer.clone(),
        APPLICATION_NAMESPACE,
        bind,
        bind,
        bootstrappers,
        MAX_MESSAGE_SIZE,
    );
    let (mut network, mut oracle) =
        discovery::Network::new(context.with_label("symphony-network"), p2p_cfg);

    let peers: Set<ed25519::PublicKey> = Set::from_iter_dedup(
        cli.peer_seeds
            .iter()
            .map(|&s| ed25519::PrivateKey::from_seed(s).public_key()),
    );
    oracle.track(0, peers).await;

    // Single channel id for the entire demo. Job id is fixed (we only support
    // one concurrent job per agent in v1; multi-job uses the lobby+slot pool).
    // Using the on-chain-equivalent room id derivation matches MultiAgentMarket.computeRoomId.
    // For a real multi-job-aware agent we'd register all 64 slot channels per §14.
    // For simplicity here: register channel for job_id=1 (the demo job).
    let demo_job_id: u64 = 1;
    let demo_room_id = room::room_id_for_chain_job(demo_job_id);
    let channel_id = room::channel_id_from_room(&demo_room_id);
    let room_key = parse_room_key(&cli.room_key_hex)?;

    info!(
        agent = %cli.name,
        demo_job_id,
        channel_id,
        "registering chat channel for demo job"
    );
    let (sender, receiver) =
        network.register(channel_id, Quota::per_second(NZU32!(64)), MAX_BACKLOG);

    // ---- Active jobs state, shared between chain watcher + chat tasks ----
    let active: ActiveJobs = Arc::new(Mutex::new(HashMap::new()));

    // ---- Spawn chain watcher: alloy WS, watches JobAwarded ----
    {
        let active_clone = active.clone();
        let cli_rpc = cli.rpc_ws.clone();
        let cli_name = cli.name.clone();
        // tokio::spawn — alloy uses tokio. commonware-runtime is built on tokio
        // so this works in the same process as the chat layer.
        tokio::spawn(async move {
            if let Err(err) = run_chain_watcher(
                cli_rpc,
                market_addr,
                me_eth_addr,
                active_clone,
                cli_name,
            )
            .await
            {
                error!(?err, "chain watcher exited with error");
            }
        });
    }

    // ---- Spawn chat receive loop ----
    {
        let active_clone = active.clone();
        let cli_name = cli.name.clone();
        context
            .with_label("symphony-recv")
            .spawn(move |_| async move {
                run_recv_loop(receiver, room_key, demo_room_id, cli_name, active_clone).await;
            });
    }

    // ---- Spawn chat send + submitter coordinator ----
    {
        let active_clone = active.clone();
        let cli_name = cli.name.clone();
        let provider_rpc = cli.rpc_ws.clone();
        let eth_signer_clone = eth_signer.clone();
        let chat_grace = Duration::from_secs(cli.chat_grace_secs);
        let exit_after_settle = cli.exit_after_settle;
        let mut sender_owned = sender;
        context
            .with_label("symphony-coord")
            .spawn(move |_| async move {
                run_coordinator(
                    &mut sender_owned,
                    room_key,
                    demo_room_id,
                    me_eth_addr,
                    me_pubkey_hex.clone(),
                    market_addr,
                    provider_rpc,
                    eth_signer_clone,
                    active_clone,
                    cli_name,
                    chat_grace,
                    exit_after_settle,
                )
                .await;
            });
    }

    let network_handler = network.start();
    let _ = network_handler.await;
    Ok(())
}

/// Subscribe to `MultiAgentMarket.JobAwarded` and update `ActiveJobs`
/// when our address appears in the winners list.
async fn run_chain_watcher(
    rpc_ws: String,
    market_addr: Address,
    me_addr: Address,
    active: ActiveJobs,
    me_name: String,
) -> Result<()> {
    let provider = ProviderBuilder::new()
        .connect_ws(WsConnect::new(&rpc_ws))
        .await
        .context("ws connect")?;
    let head = provider.get_block_number().await?;
    info!(agent = %me_name, head, "chain watcher: connected");

    let f = Filter::new()
        .address(market_addr)
        .event_signature(JobAwarded::SIGNATURE_HASH)
        .from_block(head);
    let mut stream = provider.subscribe_logs(&f).await?.into_stream();

    while let Some(log) = stream.next().await {
        let raw = AlloyLog::new(log.address(), log.topics().to_vec(), log.data().data.clone())
            .unwrap_or_else(|| AlloyLog::new_unchecked(log.address(), Vec::new(), Default::default()));
        match JobAwarded::decode_log(&raw) {
            Ok(decoded) => {
                let job_id_u256: U256 = decoded.id;
                let job_id = u64::try_from(job_id_u256).unwrap_or_else(|_| job_id_u256.as_limbs()[0]);
                let winners = decoded.winners.clone();
                let room_id_b: FixedBytes<32> = decoded.roomId;
                let am_winner = winners.iter().any(|w| *w == me_addr);
                info!(
                    agent = %me_name,
                    job_id,
                    winners_count = winners.len(),
                    am_winner,
                    "JobAwarded received"
                );
                if !am_winner {
                    continue;
                }

                // Compute deterministic result hash all winners agree on.
                // For the demo: keccak256("symphony-demo-result-v1" || job_id_be).
                let mut packed = Vec::with_capacity(40);
                packed.extend_from_slice(b"symphony-demo-result-v1");
                packed.extend_from_slice(&job_id.to_be_bytes());
                let result_hash = keccak256(&packed).0;

                let entry = ActiveJob {
                    job_id,
                    winners,
                    room_id: room_id_b.0,
                    collected_sigs: HashMap::new(),
                    result_hash,
                    final_sent: false,
                    submitted: false,
                };
                let mut map = active.lock().expect("active poisoned");
                map.insert(job_id, entry);
                info!(
                    agent = %me_name,
                    job_id,
                    result_hash_hex = %hex::encode(result_hash),
                    "ActiveJob seeded; coordinator will sign + send Final after grace period"
                );
            }
            Err(err) => error!(?err, "decode JobAwarded failed"),
        }
    }
    Ok(())
}

/// Coordinator loop: when an active job is seeded, send Hello/Status/PartialResult,
/// then after the grace period sign + send Final. When all winners' sigs are
/// collected, the lowest-address winner submits via submitMulti.
#[allow(clippy::too_many_arguments)]
async fn run_coordinator<S: commonware_p2p::Sender>(
    sender: &mut S,
    room_key: [u8; 32],
    demo_room_id: [u8; 32],
    me_eth_addr: Address,
    me_pubkey_hex: String,
    market_addr: Address,
    rpc_ws: String,
    eth_signer: PrivateKeySigner,
    active: ActiveJobs,
    me_name: String,
    chat_grace: Duration,
    exit_after_settle: bool,
) {
    info!(agent = %me_name, "coordinator loop started");
    loop {
        // Wait until at least one active job exists.
        let job_opt = {
            let map = active.lock().expect("active poisoned");
            map.values().next().cloned()
        };
        let Some(job) = job_opt else {
            tokio::time::sleep(Duration::from_millis(200)).await;
            continue;
        };

        // Phase 1: send Hello + Status (if not already done).
        if !job.final_sent {
            send_room(
                sender,
                &room_key,
                &demo_room_id,
                &RoomMessage::Hello {
                    from_pubkey_hex: me_pubkey_hex.clone(),
                    wall_clock_ms: now_ms(),
                },
            )
            .await;
            tokio::time::sleep(Duration::from_millis(500)).await;
            send_room(
                sender,
                &room_key,
                &demo_room_id,
                &RoomMessage::Status {
                    from_pubkey_hex: me_pubkey_hex.clone(),
                    phase: format!("{me_name} computing"),
                    eta_blocks: 5,
                },
            )
            .await;

            // Phase 2: wait for grace period then sign + send Final.
            info!(
                agent = %me_name,
                job_id = job.job_id,
                grace_secs = chat_grace.as_secs(),
                "waiting grace period before Final"
            );
            tokio::time::sleep(chat_grace).await;

            // Compute result digest = keccak256(RESULT_DOMAIN || id || resultHash).
            let mut packed = Vec::new();
            packed.extend_from_slice(RESULT_DOMAIN);
            packed.extend_from_slice(&U256::from(job.job_id).to_be_bytes::<32>());
            packed.extend_from_slice(&job.result_hash);
            let digest = keccak256(&packed);

            // Sign with EVM private key. alloy returns a Signature; we need
            // the (r, s, v) bytes packed for ecrecover. The `as_bytes` on the
            // signature gives 65-byte (r||s||v) representation.
            let sig = eth_signer
                .sign_hash(&digest)
                .await
                .expect("sign digest");
            let sig_bytes = sig.as_bytes();
            let sig_hex = format!("0x{}", hex::encode(sig_bytes));

            send_room(
                sender,
                &room_key,
                &demo_room_id,
                &RoomMessage::Final {
                    from_pubkey_hex: me_pubkey_hex.clone(),
                    result_hash_hex: format!("0x{}", hex::encode(job.result_hash)),
                    signature_hex: sig_hex.clone(),
                },
            )
            .await;

            // Mark final_sent + record our own sig.
            {
                let mut map = active.lock().expect("active poisoned");
                if let Some(j) = map.get_mut(&job.job_id) {
                    j.final_sent = true;
                    j.collected_sigs.insert(me_eth_addr, sig_bytes.to_vec());
                }
            }
            info!(agent = %me_name, job_id = job.job_id, "Final sent + own sig recorded");
        }

        // Phase 3: poll for sig collection completion.
        loop {
            let (count, total, am_submitter) = {
                let map = active.lock().expect("active poisoned");
                let j = map.get(&job.job_id).cloned();
                if let Some(j) = j {
                    let lowest = j.winners.iter().min().copied().unwrap_or_default();
                    (
                        j.collected_sigs.len(),
                        j.winners.len(),
                        me_eth_addr == lowest,
                    )
                } else {
                    (0, 0, false)
                }
            };

            if count >= total && total > 0 {
                if am_submitter {
                    info!(
                        agent = %me_name,
                        job_id = job.job_id,
                        sigs = count,
                        total,
                        "all sigs collected; I'm submitter — calling submitMulti"
                    );
                    let already = {
                        let map = active.lock().expect("active poisoned");
                        map.get(&job.job_id).map(|j| j.submitted).unwrap_or(false)
                    };
                    if !already {
                        if let Err(err) = submit_multi(
                            &rpc_ws,
                            &eth_signer,
                            market_addr,
                            &active,
                            job.job_id,
                            &me_name,
                        )
                        .await
                        {
                            error!(agent = %me_name, ?err, "submitMulti failed");
                        }
                    }
                } else {
                    info!(
                        agent = %me_name,
                        job_id = job.job_id,
                        sigs = count,
                        total,
                        "all sigs collected; I'm NOT the lowest-address submitter — waiting"
                    );
                }
                break;
            }
            tokio::time::sleep(Duration::from_millis(500)).await;
        }

        if exit_after_settle {
            info!(agent = %me_name, "exit_after_settle = true; exiting after 3s grace");
            tokio::time::sleep(Duration::from_secs(3)).await;
            std::process::exit(0);
        }

        // Future jobs would loop here; v1 only handles the first.
        tokio::time::sleep(Duration::from_secs(60)).await;
    }
}

async fn submit_multi(
    rpc_ws: &str,
    eth_signer: &PrivateKeySigner,
    market_addr: Address,
    active: &ActiveJobs,
    job_id: u64,
    me_name: &str,
) -> Result<()> {
    let job = {
        let map = active.lock().expect("active poisoned");
        map.get(&job_id).cloned()
    };
    let Some(mut job) = job else {
        eyre::bail!("active job missing");
    };

    // Build the signatures vec in winner order. The contract iterates winners
    // and tries to find a matching sig — so any order works, but stable is nicer.
    let sigs: Vec<AlloyBytes> = job
        .winners
        .iter()
        .filter_map(|w| job.collected_sigs.get(w).cloned())
        .map(AlloyBytes::from)
        .collect();
    if sigs.len() != job.winners.len() {
        eyre::bail!(
            "sig count mismatch: have {} of {}",
            sigs.len(),
            job.winners.len()
        );
    }

    let wallet = EthereumWallet::from(eth_signer.clone());
    let provider = ProviderBuilder::new()
        .wallet(wallet)
        .connect_ws(WsConnect::new(rpc_ws))
        .await
        .context("submit_multi ws connect")?;
    let market = IMultiAgentMarket::new(market_addr, &provider);
    let result_hash_b: FixedBytes<32> = job.result_hash.into();

    let pending = market
        .submitMulti(U256::from(job_id), result_hash_b, sigs)
        .send()
        .await
        .context("submitMulti send")?;
    let tx_hash = *pending.tx_hash();
    let receipt = pending
        .with_required_confirmations(1)
        .get_receipt()
        .await
        .context("submitMulti receipt")?;

    if receipt.status() {
        info!(
            agent = %me_name,
            job_id,
            tx_hash = %tx_hash,
            block = receipt.block_number.unwrap_or_default(),
            "submitMulti CONFIRMED"
        );
        job.submitted = true;
        let mut map = active.lock().expect("active poisoned");
        if let Some(j) = map.get_mut(&job_id) {
            j.submitted = true;
        }
    } else {
        error!(
            agent = %me_name,
            job_id,
            tx_hash = %tx_hash,
            "submitMulti REVERTED"
        );
    }
    Ok(())
}

async fn run_recv_loop<R: commonware_p2p::Receiver>(
    mut receiver: R,
    room_key: [u8; 32],
    demo_room_id: [u8; 32],
    me_name: String,
    active: ActiveJobs,
) {
    loop {
        match receiver.recv().await {
            Ok((peer, wire)) => {
                let peer_hex = hex::encode(peer.as_ref());
                let plaintext = match room::decrypt(&room_key, &demo_room_id, wire.as_ref()) {
                    Some(pt) => pt,
                    None => continue,
                };
                let msg: RoomMessage = match serde_json::from_slice(&plaintext) {
                    Ok(m) => m,
                    Err(err) => {
                        warn!(?err, "malformed room message");
                        continue;
                    }
                };
                info!(
                    agent = %me_name,
                    from_peer = %short(&peer_hex),
                    kind = msg.label(),
                    "RX"
                );

                if let RoomMessage::Final {
                    from_pubkey_hex: _,
                    result_hash_hex,
                    signature_hex,
                } = &msg
                {
                    // Match the result hash to the active job. Any active job
                    // with matching result_hash records the sig.
                    let result_hash = match parse_hex_32(result_hash_hex) {
                        Ok(h) => h,
                        Err(_) => {
                            warn!(agent = %me_name, "bad result_hash in Final");
                            continue;
                        }
                    };
                    let sig = match parse_hex_bytes(signature_hex) {
                        Ok(b) => b,
                        Err(_) => {
                            warn!(agent = %me_name, "bad signature in Final");
                            continue;
                        }
                    };

                    // Recover the signer address from the digest+sig — that's
                    // the EVM address whose sig we record.
                    let mut map = active.lock().expect("active poisoned");
                    for j in map.values_mut() {
                        if j.result_hash != result_hash {
                            continue;
                        }
                        // digest = keccak256(RESULT_DOMAIN || id_be || result_hash)
                        let mut packed = Vec::new();
                        packed.extend_from_slice(RESULT_DOMAIN);
                        packed.extend_from_slice(&U256::from(j.job_id).to_be_bytes::<32>());
                        packed.extend_from_slice(&j.result_hash);
                        let digest = keccak256(&packed);
                        if sig.len() != 65 {
                            warn!(agent = %me_name, "Final sig wrong length");
                            continue;
                        }
                        let sig_arr: [u8; 65] = sig.clone().try_into().expect("len 65 checked");
                        let alloy_sig = match alloy::signers::Signature::from_raw(&sig_arr) {
                            Ok(s) => s,
                            Err(err) => {
                                warn!(agent = %me_name, ?err, "sig parse failed");
                                continue;
                            }
                        };
                        let recovered = match alloy_sig.recover_address_from_prehash(&digest) {
                            Ok(a) => a,
                            Err(err) => {
                                warn!(agent = %me_name, ?err, "recover failed");
                                continue;
                            }
                        };
                        if !j.winners.contains(&recovered) {
                            debug!(
                                agent = %me_name,
                                recovered = %recovered,
                                "Final sig from non-winner; ignoring"
                            );
                            continue;
                        }
                        j.collected_sigs.insert(recovered, sig.clone());
                        info!(
                            agent = %me_name,
                            from_eth = %recovered,
                            job_id = j.job_id,
                            sigs_total = j.collected_sigs.len(),
                            of = j.winners.len(),
                            "sig recorded from peer Final"
                        );
                    }
                }
            }
            Err(err) => {
                debug!(?err, "recv error");
            }
        }
    }
}

async fn send_room<S: commonware_p2p::Sender>(
    sender: &mut S,
    room_key: &[u8; 32],
    room_id: &[u8; 32],
    msg: &RoomMessage,
) {
    let plaintext = serde_json::to_vec(msg).expect("serialize");
    let wire = room::encrypt(room_key, room_id, &plaintext);
    match sender.send(Recipients::All, wire, false).await {
        Ok(_) => {}
        Err(err) => warn!(?err, kind = msg.label(), "tx failed"),
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
            let seed: u64 = seed_str.parse().context("bad seed")?;
            let addr = SocketAddr::from_str(addr_str).context("bad addr")?;
            Ok((ed25519::PrivateKey::from_seed(seed).public_key(), addr.into()))
        })
        .collect()
}

fn parse_room_key(s: &str) -> Result<[u8; 32]> {
    let trimmed = s.trim().strip_prefix("0x").unwrap_or(s.trim());
    let raw = hex::decode(trimmed).context("room key hex decode")?;
    if raw.len() != 32 {
        eyre::bail!("room key must be 32 bytes");
    }
    let mut out = [0u8; 32];
    out.copy_from_slice(&raw);
    Ok(out)
}

fn parse_hex_32(s: &str) -> Result<[u8; 32]> {
    let trimmed = s.trim().strip_prefix("0x").unwrap_or(s.trim());
    let raw = hex::decode(trimmed).context("hex32")?;
    if raw.len() != 32 {
        eyre::bail!("expected 32 bytes");
    }
    let mut out = [0u8; 32];
    out.copy_from_slice(&raw);
    Ok(out)
}

fn parse_hex_bytes(s: &str) -> Result<Vec<u8>> {
    let trimmed = s.trim().strip_prefix("0x").unwrap_or(s.trim());
    Ok(hex::decode(trimmed).context("hex decode")?)
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
