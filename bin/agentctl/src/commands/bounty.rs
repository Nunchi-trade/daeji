//! `agentctl bounty` — bounty token operations: mint, approve, escrow.
//!
//! On devnet/anvil where the DAEJI token is `MockERC20`, `mint` actually
//! mints. On production, `mint` is a no-op (the token has no public mint)
//! and the operator must already hold balance.

use crate::{
    abi::{IMockERC20, IMultiAgentMarket},
    config::Config,
};
use alloy::{
    network::EthereumWallet,
    primitives::{Address, U256},
    providers::ProviderBuilder,
};
use clap::{Args as ClapArgs, Subcommand, ValueEnum};
use eyre::{Context, Result};

#[derive(Debug, ClapArgs)]
pub struct Args {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Debug, Subcommand)]
enum Cmd {
    /// Mint + approve in one step. Useful before posting a job.
    /// Mints to the configured account and approves the chosen target.
    Fund {
        /// Amount in wei (e.g. 300000000000000000000 for 300e18).
        #[arg(long)]
        amount: U256,
        /// Which contract to approve. Default: multi-agent-market.
        #[arg(long, value_enum, default_value = "multi-agent-market")]
        target: Target,
    },
    /// Show DAEJI balance + allowance to a target contract.
    Status {
        #[arg(long, value_enum, default_value = "multi-agent-market")]
        target: Target,
    },
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum Target {
    MultiAgentMarket,
    IsfrBountyPool,
}

pub async fn run(cfg: &Config, args: Args) -> Result<()> {
    let token = cfg
        .addrs
        .daeji_token
        .ok_or_else(|| eyre::eyre!("daeji_token address not set"))?;

    let wallet = EthereumWallet::from(cfg.signer.clone());
    let provider = ProviderBuilder::new()
        .wallet(wallet)
        .connect_http(cfg.rpc.parse().context("rpc parse")?);

    let me = cfg.signer.address();
    let token_iface = IMockERC20::new(token, &provider);

    match args.cmd {
        Cmd::Fund { amount, target } => {
            let target_addr = resolve_target(cfg, target)?;
            // Mint (no-op on chains without a public mint; we tolerate revert).
            match token_iface.mint(me, amount).send().await {
                Ok(pending) => {
                    let _ = pending.with_required_confirmations(1).get_receipt().await;
                    println!("[fund] minted {amount} to {me:#x}");
                }
                Err(err) => {
                    println!("[fund] mint not available on this chain (ok if production): {err:?}");
                }
            }
            // Approve.
            let pending = token_iface
                .approve(target_addr, amount)
                .send()
                .await
                .context("approve send")?;
            let receipt = pending.with_required_confirmations(1).get_receipt().await?;
            if !receipt.status() {
                eyre::bail!("approve reverted");
            }
            println!("[fund] approved {amount} → {target_addr:#x}");
            println!("       bounty escrow ready; you can now `agentctl symphony post --auto-fund=false`");
        }
        Cmd::Status { target } => {
            let target_addr = resolve_target(cfg, target)?;
            let bal = token_iface.balanceOf(me).call().await?;
            println!("DAEJI balance     : {bal}");
            println!("(allowance to {target_addr:#x} not exposed by IERC20 here; check on-chain via cast)");
        }
    }
    Ok(())
}

fn resolve_target(cfg: &Config, target: Target) -> Result<Address> {
    match target {
        Target::MultiAgentMarket => cfg
            .addrs
            .multi_agent_market
            .ok_or_else(|| eyre::eyre!("multi_agent_market address not set")),
        Target::IsfrBountyPool => cfg
            .addrs
            .isfr_bounty_pool
            .ok_or_else(|| eyre::eyre!("isfr_bounty_pool address not set")),
    }
}

/// Fund the multi-agent-market with the given amount + bp_target. Helper used
/// by symphony::post when --auto-fund is set.
pub async fn auto_fund(cfg: &Config, amount: U256) -> Result<()> {
    let token = cfg
        .addrs
        .daeji_token
        .ok_or_else(|| eyre::eyre!("daeji_token address not set"))?;
    let target = cfg
        .addrs
        .multi_agent_market
        .ok_or_else(|| eyre::eyre!("multi_agent_market address not set"))?;
    let wallet = EthereumWallet::from(cfg.signer.clone());
    let provider = ProviderBuilder::new()
        .wallet(wallet)
        .connect_http(cfg.rpc.parse().context("rpc parse")?);
    let token_iface = IMockERC20::new(token, &provider);
    let me = cfg.signer.address();

    if let Ok(pending) = token_iface.mint(me, amount).send().await {
        let _ = pending.with_required_confirmations(1).get_receipt().await;
    }
    let pending = token_iface.approve(target, amount).send().await?;
    let _ = pending.with_required_confirmations(1).get_receipt().await?;
    Ok(())
}
