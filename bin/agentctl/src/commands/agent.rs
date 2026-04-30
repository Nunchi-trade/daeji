//! `agentctl agent` — agent identity registration in AgentRegistry.

use crate::{abi::IAgentRegistry, config::Config};
use alloy::{
    network::EthereumWallet,
    primitives::{keccak256, FixedBytes},
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
    /// Register the configured account as an agent.
    Register {
        /// Capability tags, e.g. "compute|symphony|isfr-lending".
        #[arg(long)]
        capabilities: String,
        /// Optional content for the off-chain status card. The on-chain
        /// passportHash is `keccak256(content)`. Caller is expected to host
        /// the same content at `endpoint=URL` referenced in capabilities.
        #[arg(long, default_value = "demo-passport-content")]
        passport_content: String,
    },
    /// Check whether an address is currently active in AgentRegistry.
    IsActive {
        #[arg(long)]
        address: alloy::primitives::Address,
    },
}

pub async fn run(cfg: &Config, args: Args) -> Result<()> {
    let agent_registry = cfg
        .addrs
        .agent_registry
        .ok_or_else(|| eyre::eyre!("agent_registry address not set; check ~/.daeji/config.toml"))?;
    let wallet = EthereumWallet::from(cfg.signer.clone());
    let provider = ProviderBuilder::new()
        .wallet(wallet)
        .connect_http(cfg.rpc.parse().context("rpc parse")?);
    let registry = IAgentRegistry::new(agent_registry, &provider);

    match args.cmd {
        Cmd::Register { capabilities, passport_content } => {
            let hash: FixedBytes<32> = keccak256(passport_content.as_bytes());
            let pending = registry
                .register(capabilities.clone(), hash)
                .send()
                .await
                .context("register tx send")?;
            let tx_hash = *pending.tx_hash();
            let receipt = pending
                .with_required_confirmations(1)
                .get_receipt()
                .await
                .context("register receipt")?;
            if !receipt.status() {
                eyre::bail!("register tx reverted: {tx_hash:#x}");
            }
            println!("registered agent");
            println!("  account       : {:#x}", cfg.signer.address());
            println!("  capabilities  : {}", capabilities);
            println!("  passportHash  : 0x{}", hex::encode(hash));
            println!("  tx_hash       : {:#x}", tx_hash);
            println!("  block         : {}", receipt.block_number.unwrap_or_default());
        }
        Cmd::IsActive { address } => {
            let active = registry.isActive(address).call().await?;
            println!("agent {address:#x} active={active}");
        }
    }
    Ok(())
}
