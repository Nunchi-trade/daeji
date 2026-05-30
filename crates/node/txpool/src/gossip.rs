//! Scaffolding for future cross-node transaction gossip.
//!
//! Phase 1 wires the intra-node plumbing: a channel that carries raw
//! EIP-2718 encoded transactions from whatever code path accepts them
//! (RPC submission, P2P inbound) to the outbound broadcaster.
//!
//! Phase 2 (not implemented here) will add:
//! - Cross-shard relay: forward accepted transactions to peer validators
//!   that do not yet have a copy of the transaction.
//! - Per-peer rate limiting: prevent a single peer flooding the local node.
//! - Global dedup across shards: avoid redundant re-validation when the same
//!   transaction arrives on multiple channels simultaneously.
//! - Priority scheduling: prefer high-fee transactions when the broadcast
//!   channel is saturated.

use alloy_primitives::Bytes;
use tokio::sync::mpsc;

/// Default capacity of the outbound gossip channel.
///
/// At typical transaction sizes (~200 bytes) this reserves ~800 KB.
const DEFAULT_GOSSIP_CHANNEL_CAPACITY: usize = 4_096;

/// One end of the intra-node gossip pipe.
///
/// Callers that have accepted a transaction (either via RPC or from a peer)
/// send raw EIP-2718 bytes here.  The broadcaster task at the other end reads
/// from [`GossipReceiver`] and forwards to the P2P layer.
#[derive(Debug)]
pub struct GossipSender(mpsc::Sender<Bytes>);

impl GossipSender {
    /// Enqueue a raw EIP-2718 transaction for gossip broadcast.
    ///
    /// Returns `Err` if the receiver half has been dropped (i.e., the
    /// broadcaster task has shut down).  The caller should log and ignore
    /// send errors on the shutdown path.
    pub async fn send(&self, raw: Bytes) -> Result<(), mpsc::error::SendError<Bytes>> {
        self.0.send(raw).await
    }

    /// Non-blocking variant — returns immediately if the channel is full.
    ///
    /// Useful when the caller cannot await (e.g., inside a sync context).
    pub fn try_send(&self, raw: Bytes) -> Result<(), mpsc::error::TrySendError<Bytes>> {
        self.0.try_send(raw)
    }
}

/// Receiving end of the intra-node gossip pipe.
///
/// The broadcaster task holds this and drains it in a loop, forwarding
/// transactions to the P2P transport.
#[derive(Debug)]
pub struct GossipReceiver(mpsc::Receiver<Bytes>);

impl GossipReceiver {
    /// Receive the next raw transaction to broadcast.
    ///
    /// Returns `None` when all [`GossipSender`]s have been dropped.
    pub async fn recv(&mut self) -> Option<Bytes> {
        self.0.recv().await
    }
}

/// Create a new gossip pipe with the default channel capacity.
///
/// The sender half is typically held by the RPC submission path and the P2P
/// inbound handler; the receiver half is owned by the outbound broadcaster
/// task.
///
/// # Example
///
/// ```rust,ignore
/// let (tx, rx) = kora_txpool::gossip_pipe();
/// // Pass `tx` into the RPC handler and P2P inbound path.
/// // Pass `rx` into the outbound broadcast task.
/// ```
#[must_use]
pub fn gossip_pipe() -> (GossipSender, GossipReceiver) {
    gossip_pipe_with_capacity(DEFAULT_GOSSIP_CHANNEL_CAPACITY)
}

/// Create a new gossip pipe with an explicit channel capacity.
#[must_use]
pub fn gossip_pipe_with_capacity(capacity: usize) -> (GossipSender, GossipReceiver) {
    let (tx, rx) = mpsc::channel(capacity);
    (GossipSender(tx), GossipReceiver(rx))
}
