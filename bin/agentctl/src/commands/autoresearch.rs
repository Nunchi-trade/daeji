//! `agentctl autoresearch` — Predictive-Foraging job class (Phase η.1.a).
//!
//! Phase η.1.a covers:
//!  - Registering the canonical `autoresearch` jobType in `JobTypeRegistry`.
//!  - Posting a multi-agent autoresearch job via `MultiAgentMarket`.
//!
//! The matching engine (PredictionEngine, AttentionState, ActionGate) lifted
//! from `~/evm-specpool-impl/specpool-evm/src/networking/prediction.rs` lands
//! in η.1.b — symphony-agent will spawn it once a job is awarded.

use crate::{
    abi::{IJobTypeRegistry, IMultiAgentMarket},
    commands::bounty,
    config::Config,
};
use alloy::{
    network::EthereumWallet,
    primitives::{keccak256, FixedBytes, U256},
    providers::ProviderBuilder,
};
use clap::{Args as ClapArgs, Subcommand};
use daeji_autoresearch::{
    AUTORESEARCH_DESCRIPTION, AUTORESEARCH_JOB_TYPE_KEY, AUTORESEARCH_MIN_TIER,
};
use eyre::{Context, Result};

#[derive(Debug, ClapArgs)]
pub struct Args {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Debug, Subcommand)]
enum Cmd {
    /// Register the canonical `autoresearch` jobType in JobTypeRegistry.
    RegisterJobType {
        /// Min worker tier (default 2 = Standard).
        #[arg(long, default_value_t = AUTORESEARCH_MIN_TIER)]
        min_tier: u8,
        /// Min bounty floor in wei (default 50 DAEJI).
        #[arg(long, default_value = "50000000000000000000")]
        min_bounty: U256,
        /// Max deadline offset in seconds (default 1 hour).
        #[arg(long, default_value = "3600")]
        max_deadline_offset: u64,
        /// Optional off-chain metadata URI.
        #[arg(long, default_value = "")]
        metadata_uri: String,
    },
    /// Post an autoresearch job. Spec hash carries the prediction-target
    /// payload (off-chain); winners run the residual-corrected prediction
    /// loop and submit a result hash via the symphony chat protocol.
    Post {
        /// Free-form prediction target descriptor (keccak256'd into spec hash).
        /// E.g. `"market:ETH-USD"`, `"funding:0xaabb..."`, `"gas:1"`.
        #[arg(long)]
        target: String,
        /// Bounty in wei.
        #[arg(long)]
        bounty: U256,
        /// Number of agents to award.
        #[arg(long, default_value = "3")]
        num_agents: u8,
        /// Mint + approve the bounty before posting.
        #[arg(long, default_value_t = false)]
        auto_fund: bool,
    },
}

pub async fn run(cfg: &Config, args: Args) -> Result<()> {
    let wallet = EthereumWallet::from(cfg.signer.clone());
    let provider = ProviderBuilder::new()
        .wallet(wallet)
        .connect_http(cfg.rpc.parse().context("rpc parse")?);

    match args.cmd {
        Cmd::RegisterJobType { min_tier, min_bounty, max_deadline_offset, metadata_uri } => {
            let reg = cfg
                .addrs
                .job_type_registry
                .ok_or_else(|| eyre::eyre!("job_type_registry address not set"))?;
            let registry = IJobTypeRegistry::new(reg, &provider);
            let job_type: FixedBytes<32> = keccak256(AUTORESEARCH_JOB_TYPE_KEY);
            let pending = registry
                .register(
                    job_type,
                    AUTORESEARCH_DESCRIPTION.to_string(),
                    min_tier,
                    min_bounty,
                    max_deadline_offset,
                    metadata_uri.clone(),
                )
                .send()
                .await
                .context("register autoresearch job type")?;
            let tx_hash = *pending.tx_hash();
            let receipt = pending.with_required_confirmations(1).get_receipt().await?;
            if !receipt.status() {
                eyre::bail!("register reverted: {tx_hash:#x}");
            }
            println!("registered autoresearch jobType");
            println!("  jobType (bytes32) : 0x{}", hex::encode(job_type));
            println!("  description       : {AUTORESEARCH_DESCRIPTION}");
            println!("  min_tier          : {min_tier}");
            println!("  min_bounty        : {min_bounty}");
            println!("  max_deadline_off  : {max_deadline_offset}s");
            println!("  metadata_uri      : {:?}", metadata_uri);
            println!("  tx_hash           : {tx_hash:#x}");
        }
        Cmd::Post { target, bounty: bounty_amt, num_agents, auto_fund } => {
            let mam = cfg
                .addrs
                .multi_agent_market
                .ok_or_else(|| eyre::eyre!("multi_agent_market address not set"))?;
            let market = IMultiAgentMarket::new(mam, &provider);

            let spec_hash: FixedBytes<32> =
                keccak256(format!("autoresearch/{target}").as_bytes());
            let deadline = {
                use std::time::{SystemTime, UNIX_EPOCH};
                let now = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .map(|d| d.as_secs())
                    .unwrap_or(0);
                now + 3600
            };

            if auto_fund {
                println!("[autoresearch post] auto-funding bounty: {bounty_amt} wei");
                bounty::auto_fund(cfg, bounty_amt).await?;
            }

            let pending = market
                .postMultiJob(spec_hash, bounty_amt, deadline, 0, num_agents)
                .send()
                .await?;
            let tx_hash = *pending.tx_hash();
            let receipt = pending.with_required_confirmations(1).get_receipt().await?;
            if !receipt.status() {
                eyre::bail!("postMultiJob reverted: {tx_hash:#x}");
            }
            println!("posted autoresearch job");
            println!("  target        : {target}");
            println!("  spec_hash     : 0x{}", hex::encode(spec_hash));
            println!("  bounty        : {bounty_amt}");
            println!("  num_agents    : {num_agents}");
            println!("  deadline      : {deadline}");
            println!("  tx_hash       : {tx_hash:#x}");
            println!("  next: agents bid; operator runs `symphony award <job_id> --winners ...` (or `--vickrey` etc)");
        }
    }
    Ok(())
}
