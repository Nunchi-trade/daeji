//! `agentctl symphony` — multi-agent symphony jobs (MultiAgentMarket).
//!
//! Phase α covered direct-pick selection (`awardJob(winners[])`). Phase γ adds
//! Random / Vickrey / FirstPrice modes — the contract picks winners from the
//! bidder pool when the job is posted with a non-Direct mode and the deadline
//! has passed.

use crate::{
    abi::IMultiAgentMarket,
    commands::bounty,
    config::Config,
};
use alloy::{
    network::EthereumWallet,
    primitives::{Address, FixedBytes, U256},
    providers::ProviderBuilder,
};
use clap::{Args as ClapArgs, Subcommand, ValueEnum};
use eyre::{Context, Result};

/// On-chain `MultiAgentMarket.SelectionMode`.
#[derive(Debug, Clone, Copy, ValueEnum, Default, PartialEq, Eq)]
pub enum SelectionMode {
    /// Poster picks winners directly via `awardJob`. Default for backwards-compat.
    #[default]
    Direct,
    /// Resolver triggers blockhash-seeded random pick from bidders post-deadline.
    Random,
    /// Resolver triggers Vickrey uniform-pay auction (top-N by rep-adjusted score; clearing = (N+1)-th lowest bid).
    Vickrey,
    /// Resolver triggers first-price pay-as-bid auction (top-N by raw price; each pays own bid).
    FirstPrice,
}

impl SelectionMode {
    const fn as_u8(self) -> u8 {
        match self {
            Self::Direct => 0,
            Self::Random => 1,
            Self::Vickrey => 2,
            Self::FirstPrice => 3,
        }
    }
}

#[derive(Debug, ClapArgs)]
pub struct Args {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Debug, Subcommand)]
enum Cmd {
    /// Post a multi-agent job. Optionally auto-funds the bounty.
    Post {
        /// Spec hash (32 bytes hex). If omitted, hashes `--spec-content` instead.
        #[arg(long)]
        spec_hash: Option<FixedBytes<32>>,
        /// Free-form content to keccak256 into the spec hash.
        #[arg(long)]
        spec_content: Option<String>,
        /// Bounty in wei.
        #[arg(long)]
        bounty: U256,
        /// Deadline as unix seconds. Defaults to now + 1 day.
        #[arg(long)]
        deadline: Option<u64>,
        /// Required-capabilities bitmask (application-defined).
        #[arg(long, default_value = "0")]
        required_capabilities: u64,
        /// Number of agents to award.
        #[arg(long)]
        num_agents: u8,
        /// Selection mode. `direct` = poster picks via `award`; `random` = resolver
        /// picks blockhash-seeded post-deadline; `vickrey` / `first-price` =
        /// resolver runs the corresponding auction post-deadline.
        #[arg(long, value_enum, default_value = "direct")]
        selection: SelectionMode,
        /// Mint + approve the bounty before posting.
        #[arg(long, default_value = "false")]
        auto_fund: bool,
    },
    /// Bid on an open job. Phase α records the bid on-chain; Phase γ adds
    /// auction selection.
    Bid {
        job_id: u64,
        /// Ask price in wei.
        #[arg(long)]
        price: U256,
        /// Estimated time-to-completion in blocks.
        #[arg(long, default_value = "50")]
        eta_blocks: u64,
    },
    /// Show job state, winners, and (if any) submissions.
    Status { job_id: u64 },
    /// Award the job. For direct-pick jobs, pass `--winners`. For Random / Vickrey /
    /// FirstPrice jobs, the resolver picks via the corresponding auction trigger;
    /// pass `--random`, `--vickrey`, or `--first-price` (mutually exclusive).
    Award {
        job_id: u64,
        /// Comma-separated winner addresses (direct-pick only).
        #[arg(long, value_delimiter = ',', conflicts_with_all = ["random", "vickrey", "first_price"])]
        winners: Vec<Address>,
        /// Trigger blockhash-random selection (job must be posted with --selection=random).
        #[arg(long, conflicts_with_all = ["winners", "vickrey", "first_price"])]
        random: bool,
        /// Trigger Vickrey auction resolution (job must be posted with --selection=vickrey).
        #[arg(long, conflicts_with_all = ["winners", "random", "first_price"])]
        vickrey: bool,
        /// Trigger first-price auction resolution (job must be posted with --selection=first-price).
        #[arg(long, conflicts_with_all = ["winners", "random", "vickrey"])]
        first_price: bool,
    },
    /// Resolve the job (resolver only). Pays winners on accept; refunds poster on reject.
    Resolve {
        job_id: u64,
        /// Accept the result.
        #[arg(long, conflicts_with = "reject")]
        accept: bool,
        /// Reject the result.
        #[arg(long)]
        reject: bool,
    },
}

pub async fn run(cfg: &Config, args: Args) -> Result<()> {
    let mam = cfg
        .addrs
        .multi_agent_market
        .ok_or_else(|| eyre::eyre!("multi_agent_market address not set"))?;
    let wallet = EthereumWallet::from(cfg.signer.clone());
    let provider = ProviderBuilder::new()
        .wallet(wallet)
        .connect_http(cfg.rpc.parse().context("rpc parse")?);
    let market = IMultiAgentMarket::new(mam, &provider);

    match args.cmd {
        Cmd::Post {
            spec_hash,
            spec_content,
            bounty,
            deadline,
            required_capabilities,
            num_agents,
            selection,
            auto_fund,
        } => {
            let spec = match (spec_hash, spec_content) {
                (Some(h), _) => h,
                (None, Some(c)) => alloy::primitives::keccak256(c.as_bytes()),
                (None, None) => alloy::primitives::keccak256(b"agentctl-default-spec"),
            };
            let deadline = deadline.unwrap_or_else(|| {
                use std::time::{SystemTime, UNIX_EPOCH};
                let now = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
                now + 86400
            });
            if auto_fund {
                println!("[post] auto-funding bounty: {bounty} wei");
                bounty::auto_fund(cfg, bounty).await?;
            }
            let pending = if selection == SelectionMode::Direct {
                market
                    .postMultiJob(spec, bounty, deadline, required_capabilities, num_agents)
                    .send()
                    .await
                    .context("postMultiJob send")?
            } else {
                market
                    .postMultiJobWithMode(
                        spec,
                        bounty,
                        deadline,
                        required_capabilities,
                        num_agents,
                        selection.as_u8(),
                    )
                    .send()
                    .await
                    .context("postMultiJobWithMode send")?
            };
            let tx_hash = *pending.tx_hash();
            let receipt = pending.with_required_confirmations(1).get_receipt().await?;
            if !receipt.status() {
                eyre::bail!("postMultiJob reverted: {tx_hash:#x}");
            }
            // Job id is contract's prior nextJobId; we don't read it back here
            // because that would need a separate call. Operator can use
            // `agentctl symphony status <job-id>` to inspect.
            println!("posted multi-agent job");
            println!("  spec_hash         : 0x{}", hex::encode(spec));
            println!("  bounty            : {bounty}");
            println!("  deadline (unix)   : {deadline}");
            println!("  num_agents        : {num_agents}");
            println!("  required_capabs   : {required_capabilities:#018b}");
            println!("  selection         : {:?}", selection);
            println!("  tx_hash           : {tx_hash:#x}");
            println!("  block             : {}", receipt.block_number.unwrap_or_default());
            let next_hint = match selection {
                SelectionMode::Direct => "agents bid; operator runs `symphony award <job_id> --winners ...`",
                SelectionMode::Random => "agents bid; after deadline, resolver runs `symphony award <job_id> --random`",
                SelectionMode::Vickrey => "agents bid; after deadline, resolver runs `symphony award <job_id> --vickrey`",
                SelectionMode::FirstPrice => "agents bid; after deadline, resolver runs `symphony award <job_id> --first-price`",
            };
            println!("  next: {next_hint}");
        }
        Cmd::Bid { job_id, price, eta_blocks } => {
            let pending = market
                .bid(U256::from(job_id), price, eta_blocks)
                .send()
                .await?;
            let tx_hash = *pending.tx_hash();
            let receipt = pending.with_required_confirmations(1).get_receipt().await?;
            if !receipt.status() {
                eyre::bail!("bid reverted: {tx_hash:#x}");
            }
            println!("bid recorded");
            println!("  agent     : {:#x}", cfg.signer.address());
            println!("  job_id    : {job_id}");
            println!("  price     : {price}");
            println!("  eta_blocks: {eta_blocks}");
            println!("  tx_hash   : {tx_hash:#x}");
        }
        Cmd::Status { job_id } => {
            let state = market.stateOf(U256::from(job_id)).call().await?;
            let label = state_label(state);
            println!("job_id      : {job_id}");
            println!("state       : {state} ({label})");
            // getWinners only valid post-Awarded.
            if state >= 2 {
                let winners = market.getWinners(U256::from(job_id)).call().await?;
                println!("winners     : {} entries", winners.len());
                for (i, w) in winners.iter().enumerate() {
                    println!("  [{i}] {w:#x}");
                }
            }
        }
        Cmd::Award { job_id, winners, random, vickrey, first_price } => {
            let id = U256::from(job_id);
            let pending = match (winners.is_empty(), random, vickrey, first_price) {
                (false, false, false, false) => {
                    market.awardJob(id, winners.clone()).send().await
                        .context("awardJob send")?
                }
                (true, true, false, false) => {
                    market.awardJobRandom(id).send().await.context("awardJobRandom send")?
                }
                (true, false, true, false) => {
                    market.resolveAuctionVickrey(id).send().await.context("resolveAuctionVickrey send")?
                }
                (true, false, false, true) => {
                    market.resolveAuctionFirstPrice(id).send().await.context("resolveAuctionFirstPrice send")?
                }
                _ => eyre::bail!("specify exactly one of --winners, --random, --vickrey, --first-price"),
            };
            let tx_hash = *pending.tx_hash();
            let receipt = pending.with_required_confirmations(1).get_receipt().await?;
            if !receipt.status() {
                eyre::bail!("award reverted: {tx_hash:#x}");
            }
            println!("awarded job");
            println!("  job_id    : {job_id}");
            if !winners.is_empty() {
                println!("  mode      : direct");
                println!("  winners   : {} entries", winners.len());
                for w in &winners {
                    println!("    {w:#x}");
                }
            } else {
                let mode = if random { "random" } else if vickrey { "vickrey" } else { "first-price" };
                println!("  mode      : {mode}");
                let picked = market.getWinners(id).call().await?;
                println!("  winners   : {} entries (auction-picked)", picked.len());
                for w in &picked {
                    println!("    {w:#x}");
                }
                if vickrey || first_price {
                    let payments = market.getPayments(id).call().await?;
                    let total: U256 = payments.iter().copied().sum();
                    println!("  payments  : total {total} wei across {} winners", payments.len());
                }
            }
            println!("  tx_hash   : {tx_hash:#x}");
            println!("  next: agents coordinate via chat; one will call submitMulti; operator runs `symphony resolve <job_id> --accept`");
        }
        Cmd::Resolve { job_id, accept, reject } => {
            let outcome = match (accept, reject) {
                (true, false) => true,
                (false, true) => false,
                _ => eyre::bail!("specify exactly one of --accept or --reject"),
            };
            let pending = market.resolve(U256::from(job_id), outcome).send().await?;
            let tx_hash = *pending.tx_hash();
            let receipt = pending.with_required_confirmations(1).get_receipt().await?;
            if !receipt.status() {
                eyre::bail!("resolve reverted: {tx_hash:#x}");
            }
            println!("resolved job_id={job_id} accepted={outcome}");
            println!("  tx_hash : {tx_hash:#x}");
        }
    }
    Ok(())
}

fn state_label(s: u8) -> &'static str {
    match s {
        0 => "None",
        1 => "Funded",
        2 => "Awarded",
        3 => "Submitted",
        4 => "Terminal",
        _ => "Unknown",
    }
}
