//! `agentctl symphony` — multi-agent symphony jobs (MultiAgentMarket).
//!
//! Phase α covers direct-pick selection (`awardJob(winners[])`). VRF random
//! and Vickrey/first-price auctions land in Phase γ once the contracts
//! support them.

use crate::{
    abi::{IMultiAgentMarket, IWorkerRegistry},
    commands::bounty,
    config::Config,
};
use alloy::{
    network::EthereumWallet,
    primitives::{Address, FixedBytes, U256},
    providers::ProviderBuilder,
};
use clap::{Args as ClapArgs, Subcommand};
use daeji_agent_qualification::{
    should_bid_on_job, AgentProfile, DisqualificationReason, JobOffer, Tier,
};
use eyre::{Context, Result};

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
    /// Assess profitability of a posted job for the configured account.
    /// Pulls bounty + deadline + required_caps from MultiAgentMarket and the
    /// caller's tier + reputation + bond from WorkerRegistry, then runs the
    /// `daeji-agent-qualification::should_bid_on_job` projection.
    Assess {
        job_id: u64,
        /// Override the local capability bitfield (defaults to all-bits-set
        /// so capability mismatch never trips during demos).
        #[arg(long)]
        capabilities: Option<u64>,
        /// Floor below which we never bid, in pu18.
        #[arg(long, default_value = "0")]
        min_reward: u128,
        /// Reject probability prior in bps. Default 500 (5%).
        #[arg(long, default_value = "500")]
        reject_probability_bps: u16,
        /// Expected bond burned on reject, in pu18. Default 0 (no slashing).
        #[arg(long, default_value = "0")]
        slashing_amount: u128,
        /// Override the share fraction. Defaults to 10000 / num_agents.
        #[arg(long)]
        expected_share_bps: Option<u16>,
    },
    /// Award the job. Direct-pick only in Phase α.
    Award {
        job_id: u64,
        /// Comma-separated winner addresses.
        #[arg(long, value_delimiter = ',')]
        winners: Vec<Address>,
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
            let pending = market
                .postMultiJob(spec, bounty, deadline, required_capabilities, num_agents)
                .send()
                .await
                .context("postMultiJob send")?;
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
            println!("  tx_hash           : {tx_hash:#x}");
            println!("  block             : {}", receipt.block_number.unwrap_or_default());
            println!("  next: agents bid; operator runs `symphony award <job_id> --winners ...`");
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
        Cmd::Assess {
            job_id,
            capabilities,
            min_reward,
            reject_probability_bps,
            slashing_amount,
            expected_share_bps,
        } => {
            let me = cfg.signer.address();
            let job = market.getJob(U256::from(job_id)).call().await?;

            // Pull tier + reputation + stake from WorkerRegistry when configured.
            // If WorkerRegistry isn't wired, fall back to a generous default
            // profile (Trusted, 7500 bps, 1000 DAEJI bond) so the assessment
            // still runs against demo deployments.
            let (tier_u8, rep_bps, stake) =
                if let Some(reg) = cfg.addrs.worker_registry {
                    let wr = IWorkerRegistry::new(reg, &provider);
                    let t = wr.tier(me).call().await.unwrap_or(0);
                    let r = wr
                        .reputationOf(me)
                        .call()
                        .await
                        .unwrap_or(U256::from(7_500));
                    // No public stake getter on the IWorkerRegistry surface in α;
                    // we approximate with MIN_BOND when unavailable.
                    let s = wr.MIN_BOND().call().await.unwrap_or(U256::ZERO);
                    (t, u32::try_from(r).unwrap_or(7_500), u128::try_from(s).unwrap_or(0))
                } else {
                    (3, 7_500u32, 1_000u128 * 1_000_000_000_000_000_000u128)
                };

            let profile = AgentProfile {
                agent_id: address_to_id(me),
                capabilities: capabilities.unwrap_or(u64::MAX),
                reputation_bps: rep_bps,
                stake_pu18: stake,
                tier: Tier::from_u8(tier_u8),
                is_tee_attested: false,
                in_flight_jobs: 0,
                max_concurrent_jobs: 5,
                min_reward_pu18: min_reward,
            };

            let n = job.numAgents.max(1) as u16;
            let share = expected_share_bps.unwrap_or(10_000 / n);
            let offer = JobOffer {
                job_id,
                bounty_pu18: u128::try_from(job.bounty).unwrap_or(u128::MAX),
                expected_share_bps: share,
                deadline_ms: u64::from(job.deadline).saturating_mul(1_000),
                required_capabilities: job.requiredCapabilities,
                min_tier: 3,
                min_reputation_bps: 0,
                min_stake_pu18: 0,
                required_tee: false,
                expected_tx_count: 2,
                expected_gas_per_tx: 200_000,
                current_basefee_wei: 1_000_000_000,
                bounty_per_native_pu18_x18: 1,
                reject_probability_bps,
                slashing_amount_on_reject_pu18: slashing_amount,
            };

            let now_ms = {
                use std::time::{SystemTime, UNIX_EPOCH};
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .map(|d| d.as_millis() as u64)
                    .unwrap_or(0)
            };

            println!("assess job_id={job_id} for {me:#x}");
            println!("  bounty                    : {}", job.bounty);
            println!("  num_agents                : {}", job.numAgents);
            println!("  deadline (unix s)         : {}", job.deadline);
            println!("  required_capabilities     : {:#018b}", job.requiredCapabilities);
            println!("  expected_share_bps        : {share}");
            println!("  reject_probability_bps    : {reject_probability_bps}");
            println!("  slashing_on_reject (pu18) : {slashing_amount}");
            println!();

            match should_bid_on_job(&profile, &offer, now_ms) {
                Ok(score) => {
                    println!("decision  : BID");
                    println!("  expected_reward_pu18   : {}", score.expected_reward_pu18);
                    println!("  expected_gas_pu18      : {}", score.expected_gas_cost_pu18);
                    println!("  expected_slashing_pu18 : {}", score.expected_slashing_pu18);
                    println!("  net_pu18 (signed)      : {}", score.net_pu18);
                    println!("  confidence (0..10000)  : {}", score.confidence);
                }
                Err(reason) => {
                    println!("decision  : SKIP");
                    println!("  reason  : {reason}");
                    print_reason_detail(&reason);
                }
            }
        }
        Cmd::Award { job_id, winners } => {
            if winners.is_empty() {
                eyre::bail!("--winners cannot be empty");
            }
            let pending = market.awardJob(U256::from(job_id), winners.clone()).send().await?;
            let tx_hash = *pending.tx_hash();
            let receipt = pending.with_required_confirmations(1).get_receipt().await?;
            if !receipt.status() {
                eyre::bail!("awardJob reverted: {tx_hash:#x}");
            }
            println!("awarded job");
            println!("  job_id    : {job_id}");
            println!("  winners   : {} entries", winners.len());
            for w in &winners {
                println!("    {w:#x}");
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

/// Embed a 20-byte address into a 32-byte agent id (right-padded with zeroes).
fn address_to_id(a: Address) -> [u8; 32] {
    let mut out = [0u8; 32];
    out[12..32].copy_from_slice(a.as_slice());
    out
}

/// Print a one-line hint for the most common skip reasons so operators don't
/// have to memorize the qualification surface.
fn print_reason_detail(reason: &DisqualificationReason) {
    use DisqualificationReason as R;
    let hint = match reason {
        R::DeadlinePassed { .. } => "the job has already expired; nothing to do",
        R::AtCapacity { .. } => "this agent is at its concurrency cap; finish in-flight work first",
        R::InsufficientTier { .. } => "register with a larger bond or accumulate reputation to advance tier",
        R::InsufficientReputation { .. } => "reputation EWMA is below the job floor",
        R::InsufficientStake { .. } => "increase WorkerRegistry bond to clear the stake floor",
        R::CapabilityMismatch { .. } => "register with the missing capability tags",
        R::TeeRequired => "TEE attestation required — provision a verified attestation",
        R::RewardTooLow { .. } => "expected share is below your --min-reward floor",
        R::NetUnprofitable { .. } => "gas + slashing risk exceed expected reward; skip or wait for lower basefee",
    };
    println!("  hint    : {hint}");
}
