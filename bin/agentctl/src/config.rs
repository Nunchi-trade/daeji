//! Deployment-registry-aware config loader for agentctl.
//!
//! Resolution order (highest priority first):
//! 1. CLI flags (--rpc, --private-key)
//! 2. Env vars (DAEJI_RPC_URL, DAEJI_KEY)
//! 3. TOML file at --config or ~/.daeji/config.toml
//! 4. Compile-time defaults (anvil RPC, anvil deployer key)

use alloy::{primitives::Address, signers::local::PrivateKeySigner};
use eyre::{Context, Result};
use serde::Deserialize;
use std::{path::PathBuf, str::FromStr};

#[derive(Debug, Clone, Deserialize, Default)]
pub struct ConfigToml {
    /// Network section (rpc + chain id).
    #[serde(default)]
    pub network: NetworkToml,

    /// Account section (private key or keystore reference).
    #[serde(default)]
    pub account: AccountToml,

    /// Contract addresses, typically derived from a Foundry broadcast file
    /// or a deployment-registry.json. Operator can also paste them here.
    #[serde(default)]
    pub contracts: ContractsToml,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct NetworkToml {
    /// HTTP RPC endpoint, e.g. http://127.0.0.1:8545.
    pub rpc: Option<String>,
    /// WebSocket RPC endpoint, e.g. ws://127.0.0.1:8545.
    pub rpc_ws: Option<String>,
    /// Chain id (e.g. 31337 for anvil, 1337 for daeji devnet).
    pub chain_id: Option<u64>,
    /// Optional path to a Foundry broadcast JSON. When set + contracts
    /// section omits an addr, agentctl reads from the broadcast.
    pub broadcast_file: Option<PathBuf>,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct AccountToml {
    /// 0x-hex private key. Insecure for production; use keystore_path instead.
    pub private_key: Option<String>,
    /// Path to a JSON keystore (encrypted). Not yet wired in v1; private_key
    /// takes precedence.
    pub keystore_path: Option<PathBuf>,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct ContractsToml {
    pub role_registry: Option<String>,
    pub agent_registry: Option<String>,
    pub worker_registry: Option<String>,
    pub multi_agent_market: Option<String>,
    pub isfr_oracle: Option<String>,
    pub isfr_bounty_pool: Option<String>,
    pub daeji_token: Option<String>,
    pub job_type_registry: Option<String>,
    pub completion_proof: Option<String>,
}

/// Resolved configuration after merging CLI / env / TOML / defaults.
#[derive(Debug, Clone)]
pub struct Config {
    pub rpc: String,
    pub rpc_ws: Option<String>,
    pub chain_id: Option<u64>,
    pub signer: PrivateKeySigner,
    pub addrs: ContractAddrs,
}

#[derive(Debug, Clone)]
pub struct ContractAddrs {
    pub role_registry: Option<Address>,
    pub agent_registry: Option<Address>,
    pub worker_registry: Option<Address>,
    pub multi_agent_market: Option<Address>,
    pub isfr_oracle: Option<Address>,
    pub isfr_bounty_pool: Option<Address>,
    pub daeji_token: Option<Address>,
    pub job_type_registry: Option<Address>,
    pub completion_proof: Option<Address>,
}

impl Config {
    pub fn load(cli: &super::Cli) -> Result<Self> {
        let toml_cfg = load_toml(cli.config.as_deref())?;

        let rpc = cli
            .rpc
            .clone()
            .or_else(|| toml_cfg.network.rpc.clone())
            .unwrap_or_else(|| "http://127.0.0.1:8545".to_string());

        let rpc_ws = toml_cfg.network.rpc_ws.clone();
        let chain_id = toml_cfg.network.chain_id;

        let key = cli
            .private_key
            .clone()
            .or_else(|| toml_cfg.account.private_key.clone())
            .unwrap_or_else(|| {
                // Anvil default account #0 — safe for local-only demos. Production
                // operators always pass --private-key or a keystore.
                "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80".to_string()
            });

        let signer: PrivateKeySigner = key
            .parse()
            .context("invalid private key (expect 0x-hex 32 bytes)")?;

        // Resolve contract addresses: TOML first; then broadcast file if set.
        let mut addrs = ContractAddrs {
            role_registry: parse_optional(&toml_cfg.contracts.role_registry, "role_registry")?,
            agent_registry: parse_optional(&toml_cfg.contracts.agent_registry, "agent_registry")?,
            worker_registry: parse_optional(
                &toml_cfg.contracts.worker_registry,
                "worker_registry",
            )?,
            multi_agent_market: parse_optional(
                &toml_cfg.contracts.multi_agent_market,
                "multi_agent_market",
            )?,
            isfr_oracle: parse_optional(&toml_cfg.contracts.isfr_oracle, "isfr_oracle")?,
            isfr_bounty_pool: parse_optional(
                &toml_cfg.contracts.isfr_bounty_pool,
                "isfr_bounty_pool",
            )?,
            daeji_token: parse_optional(&toml_cfg.contracts.daeji_token, "daeji_token")?,
            job_type_registry: parse_optional(
                &toml_cfg.contracts.job_type_registry,
                "job_type_registry",
            )?,
            completion_proof: parse_optional(
                &toml_cfg.contracts.completion_proof,
                "completion_proof",
            )?,
        };

        // Fall back to the Foundry broadcast file if specified.
        if let Some(b) = &toml_cfg.network.broadcast_file {
            fill_from_broadcast(&mut addrs, b)?;
        }

        Ok(Self { rpc, rpc_ws, chain_id, signer, addrs })
    }
}

fn parse_optional(s: &Option<String>, field: &str) -> Result<Option<Address>> {
    s.as_deref()
        .map(|v| {
            Address::from_str(v).with_context(|| format!("invalid 0x-hex addr for {field}: {v}"))
        })
        .transpose()
}

fn load_toml(explicit: Option<&std::path::Path>) -> Result<ConfigToml> {
    let path = match explicit {
        Some(p) => p.to_path_buf(),
        None => {
            let Some(home) = dirs::home_dir() else {
                return Ok(ConfigToml::default());
            };
            home.join(".daeji").join("config.toml")
        }
    };
    if !path.exists() {
        return Ok(ConfigToml::default());
    }
    let text = std::fs::read_to_string(&path)
        .with_context(|| format!("read config: {}", path.display()))?;
    toml::from_str(&text).with_context(|| format!("parse config: {}", path.display()))
}

fn fill_from_broadcast(addrs: &mut ContractAddrs, path: &std::path::Path) -> Result<()> {
    if !path.exists() {
        return Ok(());
    }
    let text = std::fs::read_to_string(path)
        .with_context(|| format!("read broadcast: {}", path.display()))?;
    let v: serde_json::Value = serde_json::from_str(&text)
        .with_context(|| format!("parse broadcast: {}", path.display()))?;

    let txs = v.get("transactions").and_then(|x| x.as_array());
    let Some(txs) = txs else { return Ok(()); };

    for tx in txs {
        let name = tx
            .get("contractName")
            .and_then(|x| x.as_str())
            .unwrap_or_default();
        let addr = tx
            .get("contractAddress")
            .and_then(|x| x.as_str())
            .and_then(|s| Address::from_str(s).ok());
        let Some(addr) = addr else { continue; };

        match name {
            "RoleRegistry" => addrs.role_registry = addrs.role_registry.or(Some(addr)),
            "AgentRegistry" => addrs.agent_registry = addrs.agent_registry.or(Some(addr)),
            "WorkerRegistry" => addrs.worker_registry = addrs.worker_registry.or(Some(addr)),
            "MultiAgentMarket" => {
                addrs.multi_agent_market = addrs.multi_agent_market.or(Some(addr));
            }
            "ISFROracle" => addrs.isfr_oracle = addrs.isfr_oracle.or(Some(addr)),
            "ISFRBountyPool" => addrs.isfr_bounty_pool = addrs.isfr_bounty_pool.or(Some(addr)),
            "MockERC20" => addrs.daeji_token = addrs.daeji_token.or(Some(addr)),
            "JobTypeRegistry" => addrs.job_type_registry = addrs.job_type_registry.or(Some(addr)),
            "CompletionProof" => addrs.completion_proof = addrs.completion_proof.or(Some(addr)),
            _ => {}
        }
    }
    Ok(())
}
