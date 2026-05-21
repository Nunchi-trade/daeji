//! In-process [`ChatTransport`] implementation.
//!
//! Backed by per-channel `tokio::sync::broadcast` queues. Several
//! [`InMemoryChat`] instances created via [`InMemoryChat::connected_pair`]
//! (or `connected_set`) share the same backing `Arc` and so see each
//! other's traffic. Use this for unit tests, the local-first symphony
//! demo, and any scenario where multiple agents live in one Tokio
//! runtime.
//!
//! For multi-process locality (multiple agent binaries on the same box,
//! no network) a follow-up `local-chat-fs` adapter writes the same frame
//! shape under `~/.nunchi/local-chat/` and uses filesystem watches.

use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

use async_trait::async_trait;
use bytes::Bytes;
use futures::StreamExt;
use tokio::sync::broadcast;
use tokio_stream::wrappers::BroadcastStream;

use crate::{ChatError, ChatFrame, ChatResult, ChatTransport, FrameStream, PeerInfo};

/// Default channel buffer depth. A subscriber that lags by more than this
/// many frames will silently drop the oldest entries (matching the
/// "lagged-consumer" semantics other transports use).
const DEFAULT_CHANNEL_CAPACITY: usize = 256;

/// Shared per-channel broadcast queue map.
type ChannelMap = Arc<Mutex<HashMap<u64, broadcast::Sender<ChatFrame>>>>;

/// Shared peer set, kept in sync across connected instances.
type PeerSet = Arc<Mutex<Vec<PeerInfo>>>;

/// In-process chat transport. See module docs.
#[derive(Debug, Clone)]
pub struct InMemoryChat {
    me: PeerInfo,
    channels: ChannelMap,
    peers: PeerSet,
    capacity: usize,
}

impl InMemoryChat {
    /// Construct a standalone transport. Useful for single-process tests
    /// that only need one identity. For multi-agent setups call
    /// [`InMemoryChat::connected_set`] instead.
    #[must_use]
    pub fn new(me: PeerInfo) -> Self {
        Self::with_capacity(me, DEFAULT_CHANNEL_CAPACITY)
    }

    /// Like [`Self::new`] but with a caller-chosen broadcast capacity.
    #[must_use]
    pub fn with_capacity(me: PeerInfo, capacity: usize) -> Self {
        let peers = Arc::new(Mutex::new(vec![me.clone()]));
        Self {
            me,
            channels: Arc::new(Mutex::new(HashMap::new())),
            peers,
            capacity,
        }
    }

    /// Construct `n` connected transports, all sharing the same backing
    /// channel + peer maps. Each gets a distinct [`PeerInfo`] generated
    /// from the supplied `names`.
    ///
    /// Typical use: spawn 4 in-process agents in a test, give each one
    /// a transport from the returned vec, and they will see each other's
    /// published frames.
    #[must_use]
    pub fn connected_set(names: &[&str]) -> Vec<Self> {
        let channels = Arc::new(Mutex::new(HashMap::new()));
        let peers: Vec<PeerInfo> = names
            .iter()
            .map(|n| PeerInfo {
                pubkey: n.as_bytes().to_vec(),
                addr: Some(format!("inmem://{n}")),
            })
            .collect();
        let peer_set = Arc::new(Mutex::new(peers.clone()));

        peers
            .into_iter()
            .map(|me| Self {
                me,
                channels: channels.clone(),
                peers: peer_set.clone(),
                capacity: DEFAULT_CHANNEL_CAPACITY,
            })
            .collect()
    }

    /// Convenience wrapper for the two-agent case.
    #[must_use]
    pub fn connected_pair(a_name: &str, b_name: &str) -> (Self, Self) {
        let mut v = Self::connected_set(&[a_name, b_name]);
        let b = v.pop().expect("two-agent set");
        let a = v.pop().expect("two-agent set");
        (a, b)
    }

    fn sender_for(&self, channel_id: u64) -> broadcast::Sender<ChatFrame> {
        let mut guard = self.channels.lock().expect("channels poisoned");
        guard
            .entry(channel_id)
            .or_insert_with(|| broadcast::channel(self.capacity).0)
            .clone()
    }
}

#[async_trait]
impl ChatTransport for InMemoryChat {
    fn subscribe(&self, channel_id: u64) -> FrameStream {
        let sender = self.sender_for(channel_id);
        let receiver = sender.subscribe();
        // BroadcastStream emits `Result<T, BroadcastStreamRecvError>`; map
        // lag errors away so consumers see a clean `Stream<Item = ChatFrame>`.
        let stream = BroadcastStream::new(receiver).filter_map(|r| async move { r.ok() });
        Box::pin(stream)
    }

    async fn publish(&self, channel_id: u64, payload: Bytes) -> ChatResult<()> {
        let sender = self.sender_for(channel_id);
        let frame = ChatFrame {
            channel_id,
            sender: self.me.clone(),
            payload,
        };
        // `send` errors only when there are no active receivers, which is
        // fine here — fire-and-forget publish.
        let _ = sender.send(frame);
        Ok(())
    }

    async fn register_channels(&self, channels: &[u64]) -> ChatResult<()> {
        for &id in channels {
            let _ = self.sender_for(id);
        }
        Ok(())
    }

    fn peers(&self) -> Vec<PeerInfo> {
        self.peers.lock().expect("peers poisoned").clone()
    }

    fn local_identity(&self) -> PeerInfo {
        self.me.clone()
    }
}

impl ChatError {
    /// Construct an [`Other`](Self::Other) error from any displayable value.
    pub fn other(msg: impl Into<String>) -> Self {
        Self::Other(msg.into())
    }
}
