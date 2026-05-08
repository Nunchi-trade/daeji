//! WebSocket subscription support for `eth_subscribe` and `kora_subscribe`.

use jsonrpsee::{core::SubscriptionResult, proc_macros::rpc, PendingSubscriptionSink, SubscriptionMessage};
use serde::{Deserialize, Serialize};
use tokio::sync::broadcast;

use crate::types::{RpcBlock, RpcLog};

// ---------------------------------------------------------------------------
// Event types
// ---------------------------------------------------------------------------

/// Events broadcast from the finalized reporter to subscription handlers.
#[derive(Clone, Debug)]
pub enum SubscriptionEvent {
    /// A new block was finalized.
    NewHead(RpcBlock),
    /// A log was emitted in a finalized block.
    Log(RpcLog),
}

/// Consensus events broadcast by the node-state reporter.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum ConsensusEvent {
    /// A view was notarized.
    Notarization {
        /// The consensus view number.
        view: u64,
    },
    /// A view was finalized.
    Finalization {
        /// The consensus view number.
        view: u64,
    },
    /// A view was nullified.
    Nullification {
        /// The consensus view number.
        view: u64,
    },
}

// ---------------------------------------------------------------------------
// eth_subscribe / eth_unsubscribe
// ---------------------------------------------------------------------------

/// Ethereum subscription JSON-RPC API.
#[rpc(server, namespace = "eth")]
pub trait EthSubscriptionApi {
    /// Subscribe to Ethereum events.
    ///
    /// Supported kinds: `"newHeads"`, `"logs"`.
    #[subscription(name = "subscribe" => "subscription", unsubscribe = "unsubscribe", item = serde_json::Value)]
    async fn subscribe(
        &self,
        kind: String,
        params: Option<serde_json::Value>,
    ) -> SubscriptionResult;
}

/// Implementation backing `eth_subscribe`.
#[derive(Debug)]
pub struct EthSubscriptionApiImpl {
    block_broadcast: broadcast::Sender<SubscriptionEvent>,
}

impl EthSubscriptionApiImpl {
    /// Create a new implementation using the provided broadcast sender.
    pub fn new(block_broadcast: broadcast::Sender<SubscriptionEvent>) -> Self {
        Self { block_broadcast }
    }
}

#[jsonrpsee::core::async_trait]
impl EthSubscriptionApiServer for EthSubscriptionApiImpl {
    async fn subscribe(
        &self,
        pending: PendingSubscriptionSink,
        kind: String,
        params: Option<serde_json::Value>,
    ) -> SubscriptionResult {
        let mut rx = self.block_broadcast.subscribe();

        tokio::spawn(async move {
            let sink = match pending.accept().await {
                Ok(s) => s,
                Err(_) => return,
            };

            match kind.as_str() {
                "newHeads" => {
                    while let Ok(event) = rx.recv().await {
                        if let SubscriptionEvent::NewHead(block) = event {
                            let msg = SubscriptionMessage::from_json(&block)
                                .expect("RpcBlock is serializable");
                            if sink.send(msg).await.is_err() {
                                break;
                            }
                        }
                    }
                }
                "logs" => {
                    let filter = params.and_then(parse_log_filter);
                    while let Ok(event) = rx.recv().await {
                        if let SubscriptionEvent::Log(log) = event {
                            if matches_log_filter(&log, &filter) {
                                let msg = SubscriptionMessage::from_json(&log)
                                    .expect("RpcLog is serializable");
                                if sink.send(msg).await.is_err() {
                                    break;
                                }
                            }
                        }
                    }
                }
                _ => {
                    // Unknown subscription type — sink is dropped, closing the subscription.
                }
            }
        });

        Ok(())
    }
}

// ---------------------------------------------------------------------------
// kora_subscribe / kora_unsubscribe
// ---------------------------------------------------------------------------

/// Kora-specific subscription JSON-RPC API.
#[rpc(server, namespace = "kora")]
pub trait KoraSubscriptionApi {
    /// Subscribe to Kora consensus events.
    ///
    /// Supported kinds: `"consensus"`.
    #[subscription(name = "subscribe" => "subscription", unsubscribe = "unsubscribe", item = ConsensusEvent)]
    async fn subscribe(&self, kind: String) -> SubscriptionResult;
}

/// Implementation backing `kora_subscribe`.
#[derive(Debug)]
pub struct KoraSubscriptionApiImpl {
    consensus_broadcast: broadcast::Sender<ConsensusEvent>,
}

impl KoraSubscriptionApiImpl {
    /// Create a new implementation using the provided broadcast sender.
    pub fn new(consensus_broadcast: broadcast::Sender<ConsensusEvent>) -> Self {
        Self { consensus_broadcast }
    }
}

#[jsonrpsee::core::async_trait]
impl KoraSubscriptionApiServer for KoraSubscriptionApiImpl {
    async fn subscribe(
        &self,
        pending: PendingSubscriptionSink,
        kind: String,
    ) -> SubscriptionResult {
        let mut rx = self.consensus_broadcast.subscribe();

        tokio::spawn(async move {
            let sink = match pending.accept().await {
                Ok(s) => s,
                Err(_) => return,
            };

            match kind.as_str() {
                "consensus" => {
                    while let Ok(event) = rx.recv().await {
                        let msg = SubscriptionMessage::from_json(&event)
                            .expect("ConsensusEvent is serializable");
                        if sink.send(msg).await.is_err() {
                            break;
                        }
                    }
                }
                _ => {
                    // Unknown subscription type — sink is dropped.
                }
            }
        });

        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Log filter helpers
// ---------------------------------------------------------------------------

/// Parsed log subscription filter.
struct LogFilter {
    addresses: Vec<alloy_primitives::Address>,
    topics: Vec<Vec<alloy_primitives::B256>>,
}

/// Parse an optional `serde_json::Value` into a `LogFilter`.
///
/// Accepts the standard Ethereum filter object:
/// ```json
/// { "address": "0x..." | ["0x...", ...], "topics": [...] }
/// ```
fn parse_log_filter(value: serde_json::Value) -> Option<LogFilter> {
    let obj = value.as_object()?;

    let addresses = match obj.get("address") {
        Some(serde_json::Value::String(s)) => {
            vec![s.parse::<alloy_primitives::Address>().ok()?]
        }
        Some(serde_json::Value::Array(arr)) => arr
            .iter()
            .filter_map(|v| v.as_str()?.parse::<alloy_primitives::Address>().ok())
            .collect(),
        _ => Vec::new(),
    };

    let topics = match obj.get("topics") {
        Some(serde_json::Value::Array(arr)) => arr
            .iter()
            .map(|t| match t {
                serde_json::Value::Null => Vec::new(),
                serde_json::Value::String(s) => {
                    s.parse::<alloy_primitives::B256>().ok().into_iter().collect()
                }
                serde_json::Value::Array(inner) => inner
                    .iter()
                    .filter_map(|v| v.as_str()?.parse::<alloy_primitives::B256>().ok())
                    .collect(),
                _ => Vec::new(),
            })
            .collect(),
        _ => Vec::new(),
    };

    Some(LogFilter { addresses, topics })
}

/// Check whether a log matches an optional filter.
fn matches_log_filter(log: &RpcLog, filter: &Option<LogFilter>) -> bool {
    let Some(f) = filter else {
        return true;
    };

    // Address filter: if non-empty, the log address must be in the set.
    if !f.addresses.is_empty() && !f.addresses.contains(&log.address) {
        return false;
    }

    // Topic filter: AND across positions, OR within each position.
    for (idx, position) in f.topics.iter().enumerate() {
        if position.is_empty() {
            continue; // null / empty = wildcard
        }
        match log.topics.get(idx) {
            Some(log_topic) => {
                if !position.contains(log_topic) {
                    return false;
                }
            }
            None => return false,
        }
    }

    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy_primitives::{Address, B256};

    #[test]
    fn matches_log_filter_none_passes_all() {
        let log = RpcLog::default();
        assert!(matches_log_filter(&log, &None));
    }

    #[test]
    fn matches_log_filter_address_match() {
        let addr = Address::repeat_byte(0x42);
        let mut log = RpcLog::default();
        log.address = addr;

        let filter = Some(LogFilter { addresses: vec![addr], topics: Vec::new() });
        assert!(matches_log_filter(&log, &filter));
    }

    #[test]
    fn matches_log_filter_address_mismatch() {
        let mut log = RpcLog::default();
        log.address = Address::repeat_byte(0x01);

        let filter =
            Some(LogFilter { addresses: vec![Address::repeat_byte(0x02)], topics: Vec::new() });
        assert!(!matches_log_filter(&log, &filter));
    }

    #[test]
    fn matches_log_filter_topic_match() {
        let topic = B256::repeat_byte(0xab);
        let mut log = RpcLog::default();
        log.topics = vec![topic];

        let filter = Some(LogFilter { addresses: Vec::new(), topics: vec![vec![topic]] });
        assert!(matches_log_filter(&log, &filter));
    }

    #[test]
    fn matches_log_filter_topic_mismatch() {
        let mut log = RpcLog::default();
        log.topics = vec![B256::repeat_byte(0x01)];

        let filter =
            Some(LogFilter { addresses: Vec::new(), topics: vec![vec![B256::repeat_byte(0x02)]] });
        assert!(!matches_log_filter(&log, &filter));
    }

    #[test]
    fn matches_log_filter_wildcard_topic() {
        let log = RpcLog::default();

        // Empty inner vec = wildcard for that position
        let filter = Some(LogFilter { addresses: Vec::new(), topics: vec![Vec::new()] });
        assert!(matches_log_filter(&log, &filter));
    }

    #[test]
    fn parse_log_filter_address_string() {
        let val = serde_json::json!({
            "address": "0x4242424242424242424242424242424242424242"
        });
        let f = parse_log_filter(val).unwrap();
        assert_eq!(f.addresses.len(), 1);
    }

    #[test]
    fn parse_log_filter_address_array() {
        let val = serde_json::json!({
            "address": [
                "0x4242424242424242424242424242424242424242",
                "0x0101010101010101010101010101010101010101"
            ]
        });
        let f = parse_log_filter(val).unwrap();
        assert_eq!(f.addresses.len(), 2);
    }

    #[test]
    fn consensus_event_serde_roundtrip() {
        let event = ConsensusEvent::Finalization { view: 42 };
        let json = serde_json::to_string(&event).unwrap();
        assert!(json.contains("\"type\":\"finalization\""));
        assert!(json.contains("\"view\":42"));
        let parsed: ConsensusEvent = serde_json::from_str(&json).unwrap();
        assert!(matches!(parsed, ConsensusEvent::Finalization { view: 42 }));
    }
}
