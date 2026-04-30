//! `agentctl daemon` — JSON-RPC over stdio for IDE/editor integration.
//!
//! Reads newline-delimited JSON-RPC 2.0 requests on stdin, writes responses
//! to stdout. One request per line. Methods mirror the CLI subcommands 1:1
//! (e.g. `symphony.post`, `agent.register`). The same Config used by the CLI
//! drives the chain interactions, so editor wrappers see the same deployment
//! state as the operator's terminal.
//!
//! Notifications (server → client, no `id` field) are reserved for chain
//! event subscriptions (Phase ζ follow-up). The framing here supports them
//! today — emit a `JsonRpcNotification` to stdout at any time.
//!
//! Spec: https://www.jsonrpc.org/specification

use crate::{
    abi::{IAgentRegistry, IJobTypeRegistry, IMockERC20, IMultiAgentMarket, IWorkerRegistry},
    config::Config,
};
use alloy::{
    network::EthereumWallet,
    primitives::{keccak256, Address, FixedBytes, U256},
    providers::ProviderBuilder,
};
use eyre::{Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

#[derive(Debug, Deserialize)]
struct JsonRpcRequest {
    #[allow(dead_code)]
    jsonrpc: String,
    method: String,
    #[serde(default)]
    params: Value,
    id: Option<Value>,
}

#[derive(Debug, Serialize)]
struct JsonRpcResponse {
    jsonrpc: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<JsonRpcErrorBody>,
    id: Value,
}

#[derive(Debug, Serialize)]
struct JsonRpcErrorBody {
    code: i32,
    message: String,
}

const PARSE_ERROR: i32 = -32700;
const METHOD_NOT_FOUND: i32 = -32601;
const INVALID_PARAMS: i32 = -32602;
const INTERNAL_ERROR: i32 = -32000;

pub async fn run(cfg: &Config) -> Result<()> {
    let stdin = tokio::io::stdin();
    let mut lines = BufReader::new(stdin).lines();
    let mut stdout = tokio::io::stdout();

    while let Some(line) = lines.next_line().await? {
        if line.trim().is_empty() {
            continue;
        }
        let response = handle_line(cfg, &line).await;
        let serialized = serde_json::to_string(&response)?;
        stdout.write_all(serialized.as_bytes()).await?;
        stdout.write_all(b"\n").await?;
        stdout.flush().await?;
    }

    Ok(())
}

async fn handle_line(cfg: &Config, line: &str) -> JsonRpcResponse {
    let req: JsonRpcRequest = match serde_json::from_str(line) {
        Ok(r) => r,
        Err(e) => {
            return error_response(
                Value::Null,
                PARSE_ERROR,
                format!("parse error: {e}"),
            );
        }
    };
    let id = req.id.clone().unwrap_or(Value::Null);
    match dispatch(cfg, &req.method, req.params).await {
        Ok(result) => JsonRpcResponse {
            jsonrpc: "2.0",
            result: Some(result),
            error: None,
            id,
        },
        Err(DaemonError::MethodNotFound) => error_response(
            id,
            METHOD_NOT_FOUND,
            format!("method not found: {}", req.method),
        ),
        Err(DaemonError::InvalidParams(msg)) => error_response(id, INVALID_PARAMS, msg),
        Err(DaemonError::Internal(msg)) => error_response(id, INTERNAL_ERROR, msg),
    }
}

fn error_response(id: Value, code: i32, message: String) -> JsonRpcResponse {
    JsonRpcResponse {
        jsonrpc: "2.0",
        result: None,
        error: Some(JsonRpcErrorBody { code, message }),
        id,
    }
}

#[derive(Debug)]
enum DaemonError {
    MethodNotFound,
    InvalidParams(String),
    Internal(String),
}

async fn dispatch(
    cfg: &Config,
    method: &str,
    params: Value,
) -> std::result::Result<Value, DaemonError> {
    match method {
        "agent.register" => agent_register(cfg, params).await,
        "agent.is_active" => agent_is_active(cfg, params).await,
        "bounty.fund" => bounty_fund(cfg, params).await,
        "bounty.status" => bounty_status(cfg, params).await,
        "symphony.post" => symphony_post(cfg, params).await,
        "symphony.bid" => symphony_bid(cfg, params).await,
        "symphony.status" => symphony_status(cfg, params).await,
        "symphony.award" => symphony_award(cfg, params).await,
        "symphony.resolve" => symphony_resolve(cfg, params).await,
        "isfr.register_job_type" => isfr_register_job_type(cfg, params).await,
        "isfr.post_symphony" => isfr_post_symphony(cfg, params).await,
        "isfr.resolve" => isfr_resolve(cfg, params).await,
        "show.config" => show_config(cfg).await,
        _ => Err(DaemonError::MethodNotFound),
    }
}

// ───── method implementations ─────

fn invalid<S: Into<String>>(s: S) -> DaemonError {
    DaemonError::InvalidParams(s.into())
}
fn internal<S: Into<String>>(s: S) -> DaemonError {
    DaemonError::Internal(s.into())
}

fn report(e: eyre::Report) -> DaemonError {
    DaemonError::Internal(e.to_string())
}

fn parse_address(p: &Value, key: &str) -> std::result::Result<Address, DaemonError> {
    let s = p.get(key).and_then(|v| v.as_str()).ok_or_else(|| invalid(format!("missing string param `{key}`")))?;
    s.parse::<Address>().map_err(|e| invalid(format!("invalid address `{key}`: {e}")))
}

fn parse_u64(p: &Value, key: &str) -> std::result::Result<u64, DaemonError> {
    if let Some(n) = p.get(key).and_then(|v| v.as_u64()) {
        return Ok(n);
    }
    if let Some(s) = p.get(key).and_then(|v| v.as_str()) {
        return s.parse().map_err(|e| invalid(format!("invalid u64 `{key}`: {e}")));
    }
    Err(invalid(format!("missing u64 param `{key}`")))
}

fn parse_u256(p: &Value, key: &str) -> std::result::Result<U256, DaemonError> {
    if let Some(s) = p.get(key).and_then(|v| v.as_str()) {
        return s.parse().map_err(|e| invalid(format!("invalid uint256 `{key}`: {e}")));
    }
    if let Some(n) = p.get(key).and_then(|v| v.as_u64()) {
        return Ok(U256::from(n));
    }
    Err(invalid(format!("missing uint256 param `{key}`")))
}

fn parse_string(p: &Value, key: &str) -> std::result::Result<String, DaemonError> {
    p.get(key).and_then(|v| v.as_str()).map(str::to_string).ok_or_else(|| invalid(format!("missing string param `{key}`")))
}

fn parse_bytes32(p: &Value, key: &str) -> std::result::Result<FixedBytes<32>, DaemonError> {
    let s = p.get(key).and_then(|v| v.as_str()).ok_or_else(|| invalid(format!("missing bytes32 param `{key}`")))?;
    s.parse::<FixedBytes<32>>().map_err(|e| invalid(format!("invalid bytes32 `{key}`: {e}")))
}

async fn agent_register(cfg: &Config, params: Value) -> std::result::Result<Value, DaemonError> {
    let capabilities = parse_string(&params, "capabilities")?;
    let passport_content = params.get("passport_content").and_then(|v| v.as_str()).unwrap_or("daemon-passport").to_string();
    let agent_registry = cfg.addrs.agent_registry.ok_or_else(|| internal("agent_registry address not set"))?;

    let wallet = EthereumWallet::from(cfg.signer.clone());
    let provider = ProviderBuilder::new()
        .wallet(wallet)
        .connect_http(cfg.rpc.parse().map_err(|e: <alloy::transports::http::reqwest::Url as std::str::FromStr>::Err| internal(format!("rpc parse: {e}")))?);
    let registry = IAgentRegistry::new(agent_registry, &provider);

    let hash = keccak256(passport_content.as_bytes());
    let pending = registry.register(capabilities.clone(), hash).send().await.map_err(|e| internal(e.to_string()))?;
    let tx_hash = *pending.tx_hash();
    let receipt = pending.with_required_confirmations(1).get_receipt().await.map_err(|e| internal(e.to_string()))?;
    if !receipt.status() {
        return Err(internal(format!("register reverted: {tx_hash:#x}")));
    }
    Ok(json!({
        "account": format!("{:#x}", cfg.signer.address()),
        "capabilities": capabilities,
        "passport_hash": format!("0x{}", hex::encode(hash)),
        "tx_hash": format!("{tx_hash:#x}"),
        "block": receipt.block_number.unwrap_or_default(),
    }))
}

async fn agent_is_active(cfg: &Config, params: Value) -> std::result::Result<Value, DaemonError> {
    let address = parse_address(&params, "address")?;
    let agent_registry = cfg.addrs.agent_registry.ok_or_else(|| internal("agent_registry address not set"))?;
    let wallet = EthereumWallet::from(cfg.signer.clone());
    let provider = ProviderBuilder::new()
        .wallet(wallet)
        .connect_http(cfg.rpc.parse().map_err(|e: <alloy::transports::http::reqwest::Url as std::str::FromStr>::Err| internal(format!("rpc parse: {e}")))?);
    let registry = IAgentRegistry::new(agent_registry, &provider);
    let active = registry.isActive(address).call().await.map_err(|e| internal(e.to_string()))?;
    Ok(json!({ "address": format!("{address:#x}"), "active": active }))
}

async fn bounty_fund(cfg: &Config, params: Value) -> std::result::Result<Value, DaemonError> {
    let amount = parse_u256(&params, "amount")?;
    crate::commands::bounty::auto_fund(cfg, amount).await.map_err(report)?;
    Ok(json!({ "amount": amount.to_string(), "status": "minted_and_approved" }))
}

async fn bounty_status(cfg: &Config, _params: Value) -> std::result::Result<Value, DaemonError> {
    let token = cfg.addrs.daeji_token.ok_or_else(|| internal("daeji_token address not set"))?;
    let wallet = EthereumWallet::from(cfg.signer.clone());
    let provider = ProviderBuilder::new()
        .wallet(wallet)
        .connect_http(cfg.rpc.parse().map_err(|e: <alloy::transports::http::reqwest::Url as std::str::FromStr>::Err| internal(format!("rpc parse: {e}")))?);
    let token_iface = IMockERC20::new(token, &provider);
    let me = cfg.signer.address();
    let bal = token_iface.balanceOf(me).call().await.map_err(|e| internal(e.to_string()))?;
    Ok(json!({ "account": format!("{me:#x}"), "balance": bal.to_string() }))
}

async fn symphony_post(cfg: &Config, params: Value) -> std::result::Result<Value, DaemonError> {
    let mam = cfg.addrs.multi_agent_market.ok_or_else(|| internal("multi_agent_market address not set"))?;
    let wallet = EthereumWallet::from(cfg.signer.clone());
    let provider = ProviderBuilder::new()
        .wallet(wallet)
        .connect_http(cfg.rpc.parse().map_err(|e: <alloy::transports::http::reqwest::Url as std::str::FromStr>::Err| internal(format!("rpc parse: {e}")))?);
    let market = IMultiAgentMarket::new(mam, &provider);

    let bounty = parse_u256(&params, "bounty")?;
    let num_agents = parse_u64(&params, "num_agents")? as u8;
    let required_capabilities = params.get("required_capabilities").and_then(|v| v.as_u64()).unwrap_or(0);
    let deadline = params.get("deadline").and_then(|v| v.as_u64()).unwrap_or_else(|| {
        use std::time::{SystemTime, UNIX_EPOCH};
        SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0) + 86_400
    });
    let spec = match (params.get("spec_hash").and_then(|v| v.as_str()), params.get("spec_content").and_then(|v| v.as_str())) {
        (Some(h), _) => h.parse::<FixedBytes<32>>().map_err(|e| invalid(format!("spec_hash: {e}")))?,
        (None, Some(c)) => keccak256(c.as_bytes()),
        (None, None) => keccak256(b"agentctl-default-spec"),
    };
    let auto_fund = params.get("auto_fund").and_then(|v| v.as_bool()).unwrap_or(false);
    if auto_fund {
        crate::commands::bounty::auto_fund(cfg, bounty).await.map_err(report)?;
    }

    let mode = params.get("selection").and_then(|v| v.as_str()).unwrap_or("direct");
    let pending = match mode {
        "direct" => market.postMultiJob(spec, bounty, deadline, required_capabilities, num_agents).send().await,
        "random" => market.postMultiJobWithMode(spec, bounty, deadline, required_capabilities, num_agents, 1).send().await,
        "vickrey" => market.postMultiJobWithMode(spec, bounty, deadline, required_capabilities, num_agents, 2).send().await,
        "first-price" | "first_price" => market.postMultiJobWithMode(spec, bounty, deadline, required_capabilities, num_agents, 3).send().await,
        other => return Err(invalid(format!("unknown selection mode: {other}"))),
    }.map_err(|e| internal(e.to_string()))?;
    let tx_hash = *pending.tx_hash();
    let receipt = pending.with_required_confirmations(1).get_receipt().await.map_err(|e| internal(e.to_string()))?;
    if !receipt.status() {
        return Err(internal(format!("postMultiJob reverted: {tx_hash:#x}")));
    }
    Ok(json!({
        "spec_hash": format!("0x{}", hex::encode(spec)),
        "bounty": bounty.to_string(),
        "deadline": deadline,
        "num_agents": num_agents,
        "selection": mode,
        "tx_hash": format!("{tx_hash:#x}"),
        "block": receipt.block_number.unwrap_or_default(),
    }))
}

async fn symphony_bid(cfg: &Config, params: Value) -> std::result::Result<Value, DaemonError> {
    let mam = cfg.addrs.multi_agent_market.ok_or_else(|| internal("multi_agent_market address not set"))?;
    let wallet = EthereumWallet::from(cfg.signer.clone());
    let provider = ProviderBuilder::new()
        .wallet(wallet)
        .connect_http(cfg.rpc.parse().map_err(|e: <alloy::transports::http::reqwest::Url as std::str::FromStr>::Err| internal(format!("rpc parse: {e}")))?);
    let market = IMultiAgentMarket::new(mam, &provider);
    let job_id = parse_u64(&params, "job_id")?;
    let price = parse_u256(&params, "price")?;
    let eta_blocks = params.get("eta_blocks").and_then(|v| v.as_u64()).unwrap_or(50);
    let pending = market.bid(U256::from(job_id), price, eta_blocks).send().await.map_err(|e| internal(e.to_string()))?;
    let tx_hash = *pending.tx_hash();
    let receipt = pending.with_required_confirmations(1).get_receipt().await.map_err(|e| internal(e.to_string()))?;
    if !receipt.status() {
        return Err(internal(format!("bid reverted: {tx_hash:#x}")));
    }
    Ok(json!({ "job_id": job_id, "price": price.to_string(), "eta_blocks": eta_blocks, "tx_hash": format!("{tx_hash:#x}") }))
}

async fn symphony_status(cfg: &Config, params: Value) -> std::result::Result<Value, DaemonError> {
    let mam = cfg.addrs.multi_agent_market.ok_or_else(|| internal("multi_agent_market address not set"))?;
    let wallet = EthereumWallet::from(cfg.signer.clone());
    let provider = ProviderBuilder::new()
        .wallet(wallet)
        .connect_http(cfg.rpc.parse().map_err(|e: <alloy::transports::http::reqwest::Url as std::str::FromStr>::Err| internal(format!("rpc parse: {e}")))?);
    let market = IMultiAgentMarket::new(mam, &provider);
    let job_id = parse_u64(&params, "job_id")?;
    let state = market.stateOf(U256::from(job_id)).call().await.map_err(|e| internal(e.to_string()))?;
    let label = match state { 0 => "None", 1 => "Funded", 2 => "Awarded", 3 => "Submitted", 4 => "Terminal", _ => "Unknown" };
    let mut out = json!({ "job_id": job_id, "state": state, "state_label": label });
    if state >= 2 {
        let winners = market.getWinners(U256::from(job_id)).call().await.map_err(|e| internal(e.to_string()))?;
        out["winners"] = Value::Array(winners.iter().map(|w| Value::String(format!("{w:#x}"))).collect());
    }
    Ok(out)
}

async fn symphony_award(cfg: &Config, params: Value) -> std::result::Result<Value, DaemonError> {
    let mam = cfg.addrs.multi_agent_market.ok_or_else(|| internal("multi_agent_market address not set"))?;
    let wallet = EthereumWallet::from(cfg.signer.clone());
    let provider = ProviderBuilder::new()
        .wallet(wallet)
        .connect_http(cfg.rpc.parse().map_err(|e: <alloy::transports::http::reqwest::Url as std::str::FromStr>::Err| internal(format!("rpc parse: {e}")))?);
    let market = IMultiAgentMarket::new(mam, &provider);
    let id = U256::from(parse_u64(&params, "job_id")?);

    let mode = params.get("mode").and_then(|v| v.as_str()).unwrap_or("direct");
    let pending = match mode {
        "direct" => {
            let winners_arr = params.get("winners").and_then(|v| v.as_array()).ok_or_else(|| invalid("missing winners[]"))?;
            let winners: Vec<Address> = winners_arr.iter().map(|v| v.as_str().unwrap_or("").parse::<Address>().unwrap_or_default()).collect();
            market.awardJob(id, winners).send().await
        }
        "random" => market.awardJobRandom(id).send().await,
        "vickrey" => market.resolveAuctionVickrey(id).send().await,
        "first-price" | "first_price" => market.resolveAuctionFirstPrice(id).send().await,
        other => return Err(invalid(format!("unknown mode: {other}"))),
    }.map_err(|e| internal(e.to_string()))?;
    let tx_hash = *pending.tx_hash();
    let receipt = pending.with_required_confirmations(1).get_receipt().await.map_err(|e| internal(e.to_string()))?;
    if !receipt.status() {
        return Err(internal(format!("award reverted: {tx_hash:#x}")));
    }
    Ok(json!({ "job_id": id.to_string(), "mode": mode, "tx_hash": format!("{tx_hash:#x}") }))
}

async fn symphony_resolve(cfg: &Config, params: Value) -> std::result::Result<Value, DaemonError> {
    let mam = cfg.addrs.multi_agent_market.ok_or_else(|| internal("multi_agent_market address not set"))?;
    let wallet = EthereumWallet::from(cfg.signer.clone());
    let provider = ProviderBuilder::new()
        .wallet(wallet)
        .connect_http(cfg.rpc.parse().map_err(|e: <alloy::transports::http::reqwest::Url as std::str::FromStr>::Err| internal(format!("rpc parse: {e}")))?);
    let market = IMultiAgentMarket::new(mam, &provider);
    let id = U256::from(parse_u64(&params, "job_id")?);
    let accepted = params.get("accepted").and_then(|v| v.as_bool()).ok_or_else(|| invalid("missing bool `accepted`"))?;
    let pending = market.resolve(id, accepted).send().await.map_err(|e| internal(e.to_string()))?;
    let tx_hash = *pending.tx_hash();
    let receipt = pending.with_required_confirmations(1).get_receipt().await.map_err(|e| internal(e.to_string()))?;
    if !receipt.status() {
        return Err(internal(format!("resolve reverted: {tx_hash:#x}")));
    }
    Ok(json!({ "job_id": id.to_string(), "accepted": accepted, "tx_hash": format!("{tx_hash:#x}") }))
}

async fn isfr_register_job_type(cfg: &Config, params: Value) -> std::result::Result<Value, DaemonError> {
    let reg = cfg.addrs.job_type_registry.ok_or_else(|| internal("job_type_registry address not set"))?;
    let wallet = EthereumWallet::from(cfg.signer.clone());
    let provider = ProviderBuilder::new()
        .wallet(wallet)
        .connect_http(cfg.rpc.parse().map_err(|e: <alloy::transports::http::reqwest::Url as std::str::FromStr>::Err| internal(format!("rpc parse: {e}")))?);
    let registry = IJobTypeRegistry::new(reg, &provider);
    let min_tier = params.get("min_tier").and_then(|v| v.as_u64()).unwrap_or(3) as u8;
    let min_bounty: U256 = params.get("min_bounty").and_then(|v| v.as_str()).map(|s| s.parse().unwrap_or(U256::from(100u64) * U256::from(10u64).pow(U256::from(18u64)))).unwrap_or_else(|| U256::from(100u64) * U256::from(10u64).pow(U256::from(18u64)));
    let max_deadline_offset = params.get("max_deadline_offset").and_then(|v| v.as_u64()).unwrap_or(3600);
    let metadata_uri = params.get("metadata_uri").and_then(|v| v.as_str()).unwrap_or("").to_string();
    let job_type: FixedBytes<32> = keccak256(b"isfr-consensus");
    let pending = registry.register(job_type, "ISFR per-class consensus".to_string(), min_tier, min_bounty, max_deadline_offset, metadata_uri.clone()).send().await.map_err(|e| internal(e.to_string()))?;
    let tx_hash = *pending.tx_hash();
    let receipt = pending.with_required_confirmations(1).get_receipt().await.map_err(|e| internal(e.to_string()))?;
    if !receipt.status() {
        return Err(internal(format!("register reverted: {tx_hash:#x}")));
    }
    Ok(json!({
        "job_type": format!("0x{}", hex::encode(job_type)),
        "description": "ISFR per-class consensus",
        "min_tier": min_tier,
        "min_bounty": min_bounty.to_string(),
        "max_deadline_offset": max_deadline_offset,
        "metadata_uri": metadata_uri,
        "tx_hash": format!("{tx_hash:#x}"),
    }))
}

async fn isfr_post_symphony(cfg: &Config, params: Value) -> std::result::Result<Value, DaemonError> {
    let mam = cfg.addrs.multi_agent_market.ok_or_else(|| internal("multi_agent_market address not set"))?;
    let wallet = EthereumWallet::from(cfg.signer.clone());
    let provider = ProviderBuilder::new()
        .wallet(wallet)
        .connect_http(cfg.rpc.parse().map_err(|e: <alloy::transports::http::reqwest::Url as std::str::FromStr>::Err| internal(format!("rpc parse: {e}")))?);
    let market = IMultiAgentMarket::new(mam, &provider);
    let markets = parse_string(&params, "markets")?;
    let bounty = parse_u256(&params, "bounty")?;
    let num_agents = params.get("num_agents").and_then(|v| v.as_u64()).unwrap_or(4) as u8;
    let auto_fund = params.get("auto_fund").and_then(|v| v.as_bool()).unwrap_or(false);
    if auto_fund {
        crate::commands::bounty::auto_fund(cfg, bounty).await.map_err(report)?;
    }
    let spec_hash: FixedBytes<32> = keccak256(format!("isfr-consensus/{markets}").as_bytes());
    let deadline = {
        use std::time::{SystemTime, UNIX_EPOCH};
        SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0) + 3600
    };
    let pending = market.postMultiJob(spec_hash, bounty, deadline, 0, num_agents).send().await.map_err(|e| internal(e.to_string()))?;
    let tx_hash = *pending.tx_hash();
    let receipt = pending.with_required_confirmations(1).get_receipt().await.map_err(|e| internal(e.to_string()))?;
    if !receipt.status() {
        return Err(internal(format!("postMultiJob reverted: {tx_hash:#x}")));
    }
    Ok(json!({
        "markets": markets,
        "spec_hash": format!("0x{}", hex::encode(spec_hash)),
        "bounty": bounty.to_string(),
        "num_agents": num_agents,
        "deadline": deadline,
        "tx_hash": format!("{tx_hash:#x}"),
    }))
}

async fn isfr_resolve(cfg: &Config, params: Value) -> std::result::Result<Value, DaemonError> {
    let mam = cfg.addrs.multi_agent_market.ok_or_else(|| internal("multi_agent_market address not set"))?;
    let wallet = EthereumWallet::from(cfg.signer.clone());
    let provider = ProviderBuilder::new()
        .wallet(wallet)
        .connect_http(cfg.rpc.parse().map_err(|e: <alloy::transports::http::reqwest::Url as std::str::FromStr>::Err| internal(format!("rpc parse: {e}")))?);
    let market = IMultiAgentMarket::new(mam, &provider);
    let id = U256::from(parse_u64(&params, "job_id")?);
    let pending = market.resolve(id, true).send().await.map_err(|e| internal(e.to_string()))?;
    let tx_hash = *pending.tx_hash();
    let receipt = pending.with_required_confirmations(1).get_receipt().await.map_err(|e| internal(e.to_string()))?;
    if !receipt.status() {
        return Err(internal(format!("resolve reverted: {tx_hash:#x}")));
    }
    Ok(json!({ "job_id": id.to_string(), "accepted": true, "tx_hash": format!("{tx_hash:#x}") }))
}

async fn show_config(cfg: &Config) -> std::result::Result<Value, DaemonError> {
    Ok(json!({
        "rpc": cfg.rpc,
        "account": format!("{:#x}", cfg.signer.address()),
        "addresses": {
            "agent_registry": cfg.addrs.agent_registry.map(|a| format!("{a:#x}")),
            "worker_registry": cfg.addrs.worker_registry.map(|a| format!("{a:#x}")),
            "role_registry": cfg.addrs.role_registry.map(|a| format!("{a:#x}")),
            "daeji_token": cfg.addrs.daeji_token.map(|a| format!("{a:#x}")),
            "multi_agent_market": cfg.addrs.multi_agent_market.map(|a| format!("{a:#x}")),
            "job_type_registry": cfg.addrs.job_type_registry.map(|a| format!("{a:#x}")),
            "isfr_oracle": cfg.addrs.isfr_oracle.map(|a| format!("{a:#x}")),
            "isfr_bounty_pool": cfg.addrs.isfr_bounty_pool.map(|a| format!("{a:#x}")),
        },
    }))
}
