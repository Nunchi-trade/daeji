//! `agentctl isfr` — ISFR consensus jobs (symphony pattern + ISFROracle bridge).
//!
//! Phase ε wires "ISFR-as-symphony": the operator posts a multi-agent job with
//! the canonical `isfr-consensus` jobType; agents (one per source class) bid;
//! winners coordinate via chat; lowest-address winner submits a multi-sig
//! result_hash carrying the consensus rate; operator resolves and (optionally
//! in the same flow) writes the consensus to the on-chain ISFROracle.

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
use eyre::{Context, Result};

#[derive(Debug, ClapArgs)]
pub struct Args {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Debug, Subcommand)]
enum Cmd {
    /// Register the canonical `isfr-consensus` jobType in JobTypeRegistry.
    /// One-time per deployment; idempotent assuming MANAGER_ROLE.
    RegisterJobType {
        /// Min worker tier (3 = Trusted; 4 = Elite).
        #[arg(long, default_value = "3")]
        min_tier: u8,
        /// Min bounty floor in wei.
        #[arg(long, default_value = "100000000000000000000")]
        min_bounty: U256,
        /// Max deadline offset in seconds.
        #[arg(long, default_value = "3600")]
        max_deadline_offset: u64,
    },
    /// Post an ISFR-consensus job via MultiAgentMarket. 4 agents (one per
    /// class: lending / structured / funding / staking) bid, are awarded,
    /// coordinate via chat, sign + submitMulti.
    PostSymphony {
        /// Market identifier (string label, e.g. "ETH-USDC"). Hashed into the spec.
        #[arg(long)]
        markets: String,
        /// Bounty in wei.
        #[arg(long)]
        bounty: U256,
        /// Number of agents. Default 4 (one per class).
        #[arg(long, default_value = "4")]
        num_agents: u8,
        /// Mint + approve the bounty before posting.
        #[arg(long, default_value = "false")]
        auto_fund: bool,
    },
    /// Resolve an ISFR-consensus job: calls MultiAgentMarket.resolve(true).
    /// (The optional ISFROracle.submitRate bridge lands in a follow-up;
    /// the consensus rate is encoded in the job's result_hash and can be
    /// written to ISFROracle by an operator-run keeper.)
    Resolve {
        job_id: u64,
    },
}

pub async fn run(cfg: &Config, args: Args) -> Result<()> {
    match args.cmd {
        Cmd::RegisterJobType { min_tier, min_bounty, max_deadline_offset } => {
            let reg = cfg
                .addrs
                .job_type_registry
                .ok_or_else(|| eyre::eyre!("job_type_registry address not set"))?;
            let mam = cfg
                .addrs
                .multi_agent_market
                .ok_or_else(|| eyre::eyre!("multi_agent_market address not set"))?;
            // ISFROracle is optional at registration time. Deployments that
            // don't yet ship the R5 oracle stack (e.g. the symphony-demo
            // branch) can still register the canonical jobType; the metadata
            // encodes `address(0)` for the oracle and an off-chain keeper
            // bridges the consensus rate later.
            let oracle = cfg.addrs.isfr_oracle.unwrap_or(alloy::primitives::Address::ZERO);

            let wallet = EthereumWallet::from(cfg.signer.clone());
            let provider = ProviderBuilder::new()
                .wallet(wallet)
                .connect_http(cfg.rpc.parse().context("rpc parse")?);
            let registry = IJobTypeRegistry::new(reg, &provider);

            let job_type: FixedBytes<32> = keccak256(b"isfr-consensus");
            // metadata = abi.encode(MULTI_AGENT_MARKET, ISFR_ORACLE) — both
            // 20-byte addresses, padded. ISFR_ORACLE may be address(0).
            let metadata = encode_two_addrs(mam, oracle);
            let pending = registry
                .register(
                    job_type,
                    "ISFR per-class consensus".to_string(),
                    min_tier,
                    min_bounty,
                    max_deadline_offset,
                    metadata.into(),
                )
                .send()
                .await
                .context("register isfr-consensus job type")?;
            let tx_hash = *pending.tx_hash();
            let receipt = pending.with_required_confirmations(1).get_receipt().await?;
            if !receipt.status() {
                eyre::bail!("register reverted: {tx_hash:#x}");
            }
            println!("registered isfr-consensus jobType");
            println!("  jobType (bytes32) : 0x{}", hex::encode(job_type));
            println!("  description       : ISFR per-class consensus");
            println!("  min_tier          : {min_tier}");
            println!("  min_bounty        : {min_bounty}");
            println!("  max_deadline_off  : {max_deadline_offset}s");
            println!("  metadata (mam,oracle) : {mam:#x}, {oracle:#x}");
            if oracle == alloy::primitives::Address::ZERO {
                println!("  note              : oracle is address(0); deploy ISFROracle and re-register to wire the bridge");
            }
            println!("  tx_hash           : {tx_hash:#x}");
        }
        Cmd::PostSymphony { markets, bounty: bounty_amt, num_agents, auto_fund } => {
            let mam = cfg
                .addrs
                .multi_agent_market
                .ok_or_else(|| eyre::eyre!("multi_agent_market address not set"))?;
            let wallet = EthereumWallet::from(cfg.signer.clone());
            let provider = ProviderBuilder::new()
                .wallet(wallet)
                .connect_http(cfg.rpc.parse().context("rpc parse")?);
            let market = IMultiAgentMarket::new(mam, &provider);

            let spec_hash: FixedBytes<32> =
                keccak256(format!("isfr-consensus/{markets}").as_bytes());
            let deadline = {
                use std::time::{SystemTime, UNIX_EPOCH};
                let now =
                    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
                now + 3600
            };

            if auto_fund {
                println!("[post-symphony] auto-funding bounty: {bounty_amt} wei");
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
            println!("posted isfr-consensus job");
            println!("  markets       : {markets}");
            println!("  spec_hash     : 0x{}", hex::encode(spec_hash));
            println!("  bounty        : {bounty_amt}");
            println!("  num_agents    : {num_agents}");
            println!("  deadline      : {deadline}");
            println!("  tx_hash       : {tx_hash:#x}");
            println!("  next: spawn 4 symphony-agents (one per ISFR class), award them, watch chat coordination");
        }
        Cmd::Resolve { job_id } => {
            let mam = cfg
                .addrs
                .multi_agent_market
                .ok_or_else(|| eyre::eyre!("multi_agent_market address not set"))?;
            let wallet = EthereumWallet::from(cfg.signer.clone());
            let provider = ProviderBuilder::new()
                .wallet(wallet)
                .connect_http(cfg.rpc.parse().context("rpc parse")?);
            let market = IMultiAgentMarket::new(mam, &provider);

            let pending = market.resolve(U256::from(job_id), true).send().await?;
            let tx_hash = *pending.tx_hash();
            let receipt = pending.with_required_confirmations(1).get_receipt().await?;
            if !receipt.status() {
                eyre::bail!("resolve reverted: {tx_hash:#x}");
            }
            println!("resolved isfr-consensus job_id={job_id} accepted=true");
            println!("  tx_hash : {tx_hash:#x}");
            println!("  note    : on-chain ISFROracle write requires a separate keeper run; the consensus rate is encoded in the job's result_hash.");
        }
    }
    Ok(())
}

/// abi.encode(address, address) — 64 bytes, each padded to 32.
fn encode_two_addrs(a: alloy::primitives::Address, b: alloy::primitives::Address) -> Vec<u8> {
    let mut out = vec![0u8; 64];
    out[12..32].copy_from_slice(a.as_slice());
    out[44..64].copy_from_slice(b.as_slice());
    out
}
