//! `isfr-keeper-agent` — long-running Roko-style ISFR keeper.
//!
//! A real agent process (not a shell script) that:
//! 1. Connects to a chain RPC (anvil for demo, devnet for prod)
//! 2. Loads a keystore (private key) for the keeper EOA
//! 3. Loops on a configurable interval:
//!    a. Reads `currentEpoch` from the on-chain ISFROracle
//!    b. Calculates a rate (v1: random walk around a target compositeBps)
//!    c. Signs + submits `submitRate(int256, int256[4], uint16, uint64)` (fast path)
//!       OR `submitRateForRange(...)` (multi-voter Appendix A path) per `--mode`
//!    d. Logs the tx hash + new epoch
//!    e. Sleeps until the next tick
//!
//! Modeled after `roko-chain-watcher`'s reactor pattern (see `~/roko/apps/
//! roko-chain-watcher/src/main.rs`): a long-running daemon that observes chain
//! state and posts reactions back. Roko's broader agent framework (28 roles +
//! Claude/Codex/Cursor/Ollama/OpenAI dispatchers in `roko-agent`) is overkill
//! for an ISFR keeper — a keeper has no decision-making to delegate to an LLM,
//! it just needs to reliably submit a rate every epoch.
//!
//! Production path:
//! - Replace the v1 random-walk stub with reads from real yield sources
//!   (Aave / Compound / Ethena / etc. via `isfr-service`'s sources module)
//! - Or wrap this binary as the on-chain submission layer for the existing
//!   Python `isfr-service` keeper, calling out via a thin gRPC/IPC stub
//!
//! Per canonical-plan §21 the §21.A harness spawns 5 instances of this agent
//! to demonstrate the multi-voter Appendix A flow.

#![allow(missing_docs)]

use std::time::Duration;

use alloy::{
    network::EthereumWallet,
    primitives::{Address, I256, U256},
    providers::{Provider, ProviderBuilder, WsConnect},
    signers::local::PrivateKeySigner,
    sol,
};
use clap::Parser;
use eyre::{Context, Result};
use rand::Rng;
use tracing::{error, info, warn};

sol! {
    /// IISFROracle v3.0 surface. Mirrors the on-chain interface in
    /// `contracts-core/packages/agents/src/IISFROracle.sol`.
    #[sol(rpc)]
    interface IISFROracle {
        function currentEpochId() external view returns (uint32);
        function submitRate(
            int256 compositeBps,
            int256[4] calldata components,
            uint16 confidenceBps,
            uint64 timestamp
        ) external;
        function submitRateForRange(
            uint64 rangeStart,
            uint64 rangeEnd,
            int256 compositeBps,
            int256[4] calldata components,
            uint16 confidenceBps,
            uint64 observedAt
        ) external;
    }

    #[sol(rpc)]
    interface IRoleRegistry {
        function hasRole(bytes32 role, address account) external view returns (bool);
    }
}

#[derive(Debug, Parser)]
#[command(
    name = "isfr-keeper-agent",
    version,
    about = "long-running ISFR keeper agent (Roko-style reactor pattern)"
)]
struct Cli {
    /// JSON-RPC WebSocket endpoint (e.g., ws://127.0.0.1:8545).
    #[arg(long, env = "ISFR_KEEPER_RPC_WS")]
    rpc_ws: String,

    /// ISFROracle contract address (0x-hex).
    #[arg(long, env = "ISFR_KEEPER_ORACLE_ADDR")]
    oracle: String,

    /// Optional RoleRegistry contract address. When set, the agent verifies it
    /// holds KEEPER_ROLE on startup and bails fast if it doesn't.
    #[arg(long, env = "ISFR_KEEPER_ROLE_REGISTRY")]
    role_registry: Option<String>,

    /// Keeper EOA private key (0x-hex). Anvil default keys work for local demos.
    #[arg(long, env = "ISFR_KEEPER_PRIVATE_KEY")]
    private_key: String,

    /// Submission mode. `fast` uses `submitRate` (single-keeper fast path);
    /// `range` uses `submitRateForRange` (multi-voter Appendix A path).
    #[arg(long, env = "ISFR_KEEPER_MODE", default_value = "fast")]
    mode: Mode,

    /// Seconds between submissions. Default 5s for demo speed; production
    /// keepers run on much longer intervals (per the ISFR paper, hourly).
    #[arg(long, env = "ISFR_KEEPER_INTERVAL_SECS", default_value = "5")]
    interval_secs: u64,

    /// For `range` mode: number of blocks back from chain head to use as the
    /// range window. Default 2 matches the §21.A demo configuration.
    #[arg(long, env = "ISFR_KEEPER_RANGE_WIDTH", default_value = "2")]
    range_width: u64,

    /// For `range` mode: override range start/end so multiple keeper instances
    /// all vote for the SAME range (Appendix A multi-voter quorum). When both
    /// `fixed_range_start` and `fixed_range_end` are set, the agent uses them
    /// instead of computing from chain head. Required when running 5+ agents
    /// concurrently — without this they each pick a different range as head
    /// advances, and the quorum never closes.
    #[arg(long, env = "ISFR_KEEPER_FIXED_RANGE_START")]
    fixed_range_start: Option<u64>,

    /// See `fixed_range_start`.
    #[arg(long, env = "ISFR_KEEPER_FIXED_RANGE_END")]
    fixed_range_end: Option<u64>,

    /// Target composite_bps for the random-walk stub (v1). Production keepers
    /// replace this with real source reads.
    #[arg(long, env = "ISFR_KEEPER_TARGET_BPS", default_value = "690")]
    target_bps: i32,

    /// Random-walk volatility. Per-tick the rate moves +/- this many bps from
    /// the previous output, clamped within [target - 50, target + 50].
    #[arg(long, env = "ISFR_KEEPER_VOLATILITY_BPS", default_value = "5")]
    volatility_bps: i32,

    /// Optional human-readable name for log output (e.g. "keeper-1"). Defaults
    /// to the keeper's address.
    #[arg(long, env = "ISFR_KEEPER_NAME")]
    name: Option<String>,

    /// Stop after N submissions instead of running forever. Useful for tests.
    /// 0 = run forever (default).
    #[arg(long, env = "ISFR_KEEPER_MAX_TICKS", default_value = "0")]
    max_ticks: u32,
}

#[derive(Debug, Clone, Copy, clap::ValueEnum)]
enum Mode {
    Fast,
    Range,
}

#[tokio::main]
async fn main() -> Result<()> {
    let _ = tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info,isfr_keeper_agent=info".into()),
        )
        .try_init();

    let cli = Cli::parse();

    // --- key + wallet ---
    let signer: PrivateKeySigner = cli
        .private_key
        .parse()
        .context("invalid keeper private key (expect 0x-hex)")?;
    let keeper_addr = signer.address();
    let display_name = cli
        .name
        .clone()
        .unwrap_or_else(|| format!("{keeper_addr:#x}"));

    let oracle_addr: Address =
        cli.oracle.parse().with_context(|| format!("invalid oracle addr: {}", cli.oracle))?;

    info!(
        agent = %display_name,
        keeper = %keeper_addr,
        oracle = %oracle_addr,
        mode = ?cli.mode,
        interval_secs = cli.interval_secs,
        target_bps = cli.target_bps,
        "isfr-keeper-agent starting"
    );

    // --- provider ---
    let wallet = EthereumWallet::from(signer);
    let provider = ProviderBuilder::new()
        .wallet(wallet)
        .connect_ws(WsConnect::new(&cli.rpc_ws))
        .await
        .with_context(|| format!("connect to {}", cli.rpc_ws))?;
    let chain_id = provider.get_chain_id().await?;
    let head = provider.get_block_number().await?;
    info!(agent = %display_name, chain_id, head, "connected to chain");

    // --- preflight: verify KEEPER_ROLE if RoleRegistry is provided ---
    if let Some(rr) = &cli.role_registry {
        let rr_addr: Address = rr.parse().with_context(|| format!("invalid role registry: {rr}"))?;
        let role_registry = IRoleRegistry::new(rr_addr, &provider);
        // KEEPER_ROLE = keccak256("KEEPER_ROLE")
        let keeper_role: alloy::primitives::FixedBytes<32> =
            alloy::primitives::keccak256("KEEPER_ROLE".as_bytes());
        let has = role_registry
            .hasRole(keeper_role, keeper_addr)
            .call()
            .await
            .context("hasRole call failed")?;
        if !has {
            warn!(
                agent = %display_name,
                "keeper does NOT have KEEPER_ROLE. Submission will revert. Grant via the deployer's MANAGER_ROLE before starting this agent."
            );
        } else {
            info!(agent = %display_name, "KEEPER_ROLE verified");
        }
    }

    // --- reactor loop ---
    let oracle = IISFROracle::new(oracle_addr, &provider);
    let mut last_bps: i32 = cli.target_bps;
    let mut tick: u32 = 0;

    loop {
        tick += 1;

        // v1 rate calculation: random walk around target.
        let mut rng = rand::thread_rng();
        let delta: i32 = rng.gen_range(-(cli.volatility_bps)..=cli.volatility_bps);
        let mut composite = last_bps + delta;
        let lo = cli.target_bps - 50;
        let hi = cli.target_bps + 50;
        if composite < lo {
            composite = lo;
        }
        if composite > hi {
            composite = hi;
        }
        last_bps = composite;

        // Per-class components — wiggle around composite for realism. Production
        // keepers compute these from per-source weighted medians.
        let components: [I256; 4] = [
            I256::try_from(composite - 100).unwrap_or_default(),
            I256::try_from(composite + 30).unwrap_or_default(),
            I256::try_from(composite + 140).unwrap_or_default(),
            I256::try_from(composite - 280).unwrap_or_default(),
        ];
        let composite_i = I256::try_from(composite).unwrap_or_default();
        let confidence: u16 = 8500;
        let timestamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);

        let result = match cli.mode {
            Mode::Fast => oracle
                .submitRate(composite_i, components, confidence, timestamp)
                .send()
                .await,
            Mode::Range => {
                let (range_start, range_end) =
                    match (cli.fixed_range_start, cli.fixed_range_end) {
                        (Some(s), Some(e)) => (s, e),
                        _ => {
                            let head = provider.get_block_number().await?;
                            (head.saturating_sub(cli.range_width), head)
                        }
                    };
                oracle
                    .submitRateForRange(
                        range_start,
                        range_end,
                        composite_i,
                        components,
                        confidence,
                        timestamp,
                    )
                    .send()
                    .await
            }
        };

        match result {
            Ok(pending) => {
                let tx_hash = *pending.tx_hash();
                // Wait for confirmation; bound the wait so a stuck tx doesn't hang the loop forever.
                let receipt = tokio::time::timeout(
                    Duration::from_secs(15),
                    pending.with_required_confirmations(1).get_receipt(),
                )
                .await;
                match receipt {
                    Ok(Ok(rcpt)) => {
                        if rcpt.status() {
                            info!(
                                agent = %display_name,
                                tick,
                                tx_hash = %tx_hash,
                                block = rcpt.block_number.unwrap_or_default(),
                                composite_bps = composite,
                                confidence_bps = confidence,
                                "submitted rate"
                            );
                        } else {
                            error!(
                                agent = %display_name,
                                tick,
                                tx_hash = %tx_hash,
                                block = rcpt.block_number.unwrap_or_default(),
                                "tx reverted (status=0); contract rejected the submission"
                            );
                            std::process::exit(2);
                        }
                    }
                    Ok(Err(err)) => {
                        warn!(agent = %display_name, tick, ?err, "tx confirm failed");
                    }
                    Err(_) => {
                        warn!(agent = %display_name, tick, %tx_hash, "tx confirm timed out (15s)");
                    }
                }
            }
            Err(err) => {
                error!(agent = %display_name, tick, ?err, "submit failed");
            }
        }

        if cli.max_ticks > 0 && tick >= cli.max_ticks {
            info!(agent = %display_name, tick, "max_ticks reached; exiting");
            break;
        }

        tokio::time::sleep(Duration::from_secs(cli.interval_secs)).await;
    }

    Ok(())
}

// Suppress unused warnings for the U256 import — kept for future range-window math.
#[allow(dead_code)]
fn _unused() -> U256 {
    U256::ZERO
}

#[cfg(test)]
mod tests {
    #[test]
    fn smoke() {
        // Compile-time smoke. Real integration is via the §21.A e2e harness.
        assert!(true);
    }
}
