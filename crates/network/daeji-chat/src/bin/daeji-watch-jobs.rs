//! Phase-3 chain watcher.
//!
//! Subscribes to `MultiAgentMarket::JobAwarded` events on a JSON-RPC endpoint
//! (usually `anvil` for the POC, real Daeji RPC in production), derives the
//! commonware channel id for each awarded job, and prints the binding.
//!
//! This is the on-chain → off-chain glue: an agent listening to this stream knows
//! exactly which channel to register for each job it was awarded.

use alloy::{
    primitives::{Address, Bytes, FixedBytes, Log as AlloyLog, U256},
    providers::{Provider, ProviderBuilder, WsConnect},
    rpc::types::eth::Filter,
    sol,
    sol_types::SolEvent,
};
use clap::Parser;
use daeji_chat::room;
use futures::StreamExt;
use std::str::FromStr;
use tracing::{error, info, warn};

sol! {
    /// Mirror of the Solidity event signature in MultiAgentMarket.sol.
    #[derive(Debug)]
    event JobAwarded(uint256 indexed id, address[] winners, bytes32 roomId);
}

#[derive(Parser, Debug)]
#[command(version, about = "Watch MultiAgentMarket.JobAwarded events and print channel bindings.")]
struct Args {
    /// JSON-RPC WebSocket endpoint (anvil default: ws://127.0.0.1:8545).
    #[arg(long, default_value = "ws://127.0.0.1:8545")]
    rpc: String,

    /// Deployed MultiAgentMarket contract address.
    #[arg(long)]
    market: String,

    /// Block to start scanning from (default: latest).
    #[arg(long)]
    from_block: Option<u64>,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let args = Args::parse();
    let market = Address::from_str(&args.market)?;

    info!(rpc = %args.rpc, market = %market, "connecting");
    let provider = ProviderBuilder::new()
        .connect_ws(WsConnect::new(args.rpc))
        .await?;

    let chain_id = provider.get_chain_id().await?;
    let head = provider.get_block_number().await?;
    info!(chain_id, head, "connected");

    let mut filter = Filter::new()
        .address(market)
        .event_signature(JobAwarded::SIGNATURE_HASH);
    if let Some(from) = args.from_block {
        filter = filter.from_block(from);
    } else {
        filter = filter.from_block(head);
    }

    let sub = provider.subscribe_logs(&filter).await?;
    let mut stream = sub.into_stream();
    info!("subscribed — waiting for JobAwarded events…");

    while let Some(log) = stream.next().await {
        let topics = log.topics();
        let data = log.data().data.clone();
        let raw = AlloyLog::new(log.address(), topics.to_vec(), data).unwrap_or_else(|| {
            // SAFETY: this only fails on too-many-topics, which can't happen for our schema.
            AlloyLog::new_unchecked(log.address(), Vec::new(), Bytes::new())
        });

        match JobAwarded::decode_log(&raw) {
            Ok(decoded) => {
                let id: U256 = decoded.id;
                let room_id: FixedBytes<32> = decoded.roomId;
                let winners = &decoded.winners;

                let job_id_u64 = u64::try_from(id).unwrap_or_else(|_| {
                    warn!(id_hex = %id, "job_id exceeds u64 — using truncated lower 64 bits");
                    id.as_limbs()[0]
                });
                let derived_room = room::room_id_for_chain_job(job_id_u64);
                let channel_id = room::channel_id_from_room(&derived_room);

                let parity = if derived_room == room_id.0 { "✓" } else { "MISMATCH" };
                info!(
                    job_id = job_id_u64,
                    room_id_chain = %hex::encode(room_id),
                    room_id_derived = %hex::encode(derived_room),
                    parity,
                    channel_id,
                    winners = ?winners,
                    block = log.block_number.unwrap_or_default(),
                    "JobAwarded → channel binding"
                );
            }
            Err(err) => {
                error!(?err, "failed to decode JobAwarded log");
            }
        }
    }

    Ok(())
}
