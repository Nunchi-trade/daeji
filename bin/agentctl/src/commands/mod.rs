//! Subcommand dispatch.

pub mod agent;
pub mod autoresearch;
pub mod bounty;
pub mod isfr;
pub mod mining;
pub mod symphony;

use crate::config::Config;
use eyre::Result;

/// `agentctl show` — print resolved configuration + accounts.
pub fn show(cfg: &Config, json: bool) -> Result<()> {
    let me = cfg.signer.address();
    if json {
        let v = serde_json::json!({
            "rpc": cfg.rpc,
            "rpc_ws": cfg.rpc_ws,
            "chain_id": cfg.chain_id,
            "account": format!("{me:#x}"),
            "contracts": {
                "role_registry": cfg.addrs.role_registry.map(|a| format!("{a:#x}")),
                "agent_registry": cfg.addrs.agent_registry.map(|a| format!("{a:#x}")),
                "worker_registry": cfg.addrs.worker_registry.map(|a| format!("{a:#x}")),
                "multi_agent_market": cfg.addrs.multi_agent_market.map(|a| format!("{a:#x}")),
                "isfr_oracle": cfg.addrs.isfr_oracle.map(|a| format!("{a:#x}")),
                "isfr_bounty_pool": cfg.addrs.isfr_bounty_pool.map(|a| format!("{a:#x}")),
                "daeji_token": cfg.addrs.daeji_token.map(|a| format!("{a:#x}")),
                "job_type_registry": cfg.addrs.job_type_registry.map(|a| format!("{a:#x}")),
                "completion_proof": cfg.addrs.completion_proof.map(|a| format!("{a:#x}")),
            },
        });
        println!("{}", serde_json::to_string_pretty(&v)?);
    } else {
        println!("RPC          : {}", cfg.rpc);
        println!("Account      : {me:#x}");
        if let Some(a) = cfg.addrs.role_registry        { println!("RoleRegistry      : {a:#x}"); }
        if let Some(a) = cfg.addrs.agent_registry       { println!("AgentRegistry     : {a:#x}"); }
        if let Some(a) = cfg.addrs.worker_registry      { println!("WorkerRegistry    : {a:#x}"); }
        if let Some(a) = cfg.addrs.multi_agent_market   { println!("MultiAgentMarket  : {a:#x}"); }
        if let Some(a) = cfg.addrs.isfr_oracle          { println!("ISFROracle        : {a:#x}"); }
        if let Some(a) = cfg.addrs.isfr_bounty_pool     { println!("ISFRBountyPool    : {a:#x}"); }
        if let Some(a) = cfg.addrs.daeji_token          { println!("DAEJI token       : {a:#x}"); }
        if let Some(a) = cfg.addrs.job_type_registry    { println!("JobTypeRegistry   : {a:#x}"); }
        if let Some(a) = cfg.addrs.completion_proof     { println!("CompletionProof   : {a:#x}"); }
    }
    Ok(())
}
