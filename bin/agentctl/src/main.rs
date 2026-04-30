//! `agentctl` — operator + agent CLI for the Daeji agent-coordination layer.
//!
//! Wraps the contract surface so operators don't memorize Solidity ABIs:
//!   agentctl agent register --capabilities "compute|symphony|isfr-lending"
//!   agentctl bounty fund --amount 300e18
//!   agentctl symphony post --bounty 300e18 --num-agents 4 --auto-fund
//!   agentctl symphony bid <job-id> --price 80e18
//!   agentctl symphony status <job-id>
//!   agentctl symphony award <job-id> --winners 0x...,0x...
//!   agentctl symphony resolve <job-id> --accept
//!   agentctl isfr post-symphony --markets ETH-USDC --bounty 300e18
//!   agentctl isfr resolve <job-id>
//!
//! Reads addresses from `~/.daeji/config.toml` (or `--config <path>`) so the
//! same binary works against anvil / devnet / mainnet without recompiling.

#![allow(missing_docs)]

use clap::{Parser, Subcommand};
use eyre::Result;

mod abi;
mod commands;
mod config;
mod daemon;

#[derive(Debug, Parser)]
#[command(
    name = "agentctl",
    version,
    about = "Operator + agent CLI for the Daeji agent-coordination layer"
)]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,

    /// Path to config TOML (default: ~/.daeji/config.toml).
    #[arg(long, global = true)]
    config: Option<std::path::PathBuf>,

    /// JSON-RPC HTTP endpoint (overrides config). Env: DAEJI_RPC_URL.
    #[arg(long, global = true, env = "DAEJI_RPC_URL")]
    rpc: Option<String>,

    /// Private key (0x-hex). Overrides config + keystore. Env: DAEJI_KEY.
    #[arg(long, global = true, env = "DAEJI_KEY")]
    private_key: Option<String>,

    /// Emit machine-readable JSON instead of human-readable text. Useful
    /// for IDE wrappers and the upcoming agentctl daemon (Phase ζ).
    #[arg(long, global = true)]
    json: bool,
}

#[derive(Debug, Subcommand)]
enum Cmd {
    /// Agent registration in AgentRegistry.
    Agent(commands::agent::Args),
    /// Bounty token operations: mint + approve + escrow.
    Bounty(commands::bounty::Args),
    /// Symphony-class multi-agent jobs (MultiAgentMarket).
    Symphony(commands::symphony::Args),
    /// ISFR consensus jobs — symphony pattern wired to ISFROracle.
    Isfr(commands::isfr::Args),
    /// Mining-bounty jobs (Phase η.3 — placeholder for now).
    Mining(commands::mining::Args),
    /// Show resolved deployment configuration + accounts.
    Show,
    /// JSON-RPC daemon mode for IDE / editor integration. Reads
    /// newline-delimited JSON-RPC 2.0 requests on stdin, writes responses
    /// (and chain-event notifications) to stdout. Methods mirror the CLI
    /// subcommands 1:1 (e.g. `symphony.post`, `agent.register`).
    Daemon,
}

#[tokio::main]
async fn main() -> Result<()> {
    let _ = tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info,agentctl=info".into()),
        )
        .with_target(false)
        .try_init();

    let cli = Cli::parse();
    let cfg = config::Config::load(&cli)?;

    match cli.cmd {
        Cmd::Agent(args) => commands::agent::run(&cfg, args).await,
        Cmd::Bounty(args) => commands::bounty::run(&cfg, args).await,
        Cmd::Symphony(args) => commands::symphony::run(&cfg, args).await,
        Cmd::Isfr(args) => commands::isfr::run(&cfg, args).await,
        Cmd::Mining(args) => commands::mining::run(&cfg, args).await,
        Cmd::Show => commands::show(&cfg, cli.json),
        Cmd::Daemon => daemon::run(&cfg).await,
    }
}
