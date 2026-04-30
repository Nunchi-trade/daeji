//! `agentctl mining` — mining-bounty jobs (Phase η.3, placeholder for now).

use crate::config::Config;
use clap::{Args as ClapArgs, Subcommand};
use eyre::Result;

#[derive(Debug, ClapArgs)]
pub struct Args {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Debug, Subcommand)]
enum Cmd {
    /// Stub. Implemented in Phase η.3 once `MiningMarket.sol` lands.
    Post,
    /// Stub. Implemented in Phase η.3.
    Claim,
}

pub async fn run(_cfg: &Config, args: Args) -> Result<()> {
    match args.cmd {
        Cmd::Post | Cmd::Claim => {
            println!(
                "mining commands are placeholders for canonical-plan Phase η.3.\n\
                 Source to port: ~/gossip-protocol/specpool-evm/src/networking/agent_mining.rs (1971 LOC)\n\
                 Contract surface to extend: MultiAgentMarket with branch-DAG + royalty fields, OR new MiningMarket.sol."
            );
            Ok(())
        }
    }
}
