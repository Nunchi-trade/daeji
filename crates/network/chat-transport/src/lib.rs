//! Chat-transport abstraction.
//!
//! Decouples lobby + marketplace + agent-runtime code from any specific chat
//! implementation. Implementors plug a concrete transport (commonware-p2p,
//! libp2p, iroh, in-memory, filesystem mirror) behind the same
//! [`ChatTransport`] trait; consumers stay agnostic.
//!
//! # Design
//!
//! - **One channel = one numeric id** (`u64`). Channels are flat; no
//!   hierarchy. Lobby + slot pool callers derive ids from keccak domain tags
//!   (`DAEJI_LOBBY_V1`, `DAEJI_JOB_SLOT_V1`); the trait itself stays content-
//!   agnostic.
//! - **Frames are opaque bytes** ([`Bytes`]). AEAD encryption, message
//!   framing, and protocol parsing happen above this trait, in
//!   `daeji-lobby` / `symphony-agent`.
//! - **Subscribe returns a [`Stream`]** of received frames. Implementors
//!   decide buffering, lag behavior, and back-pressure.
//! - **Publish is `async`** — most real transports need to await network
//!   confirmation or backpressure. In-memory impls return immediately.
//! - **`register_channels` is a hook for transports that require pre-
//!   declaration** (commonware-p2p does; in-memory ignores).
//!
//! # Implementations in this crate
//!
//! - [`InMemoryChat`] — tokio broadcast channels, no network. The
//!   default for tests, local demos, and the "off-chain + local first"
//!   path.
//!
//! Other transports live in their own crates and depend on this one:
//! `daeji-chat` (commonware-p2p adapter), a future `iroh-chat`, etc.

mod in_memory;

pub use in_memory::InMemoryChat;

use std::pin::Pin;

use async_trait::async_trait;
use bytes::Bytes;
use futures::Stream;

/// A frame received on (or published to) a channel.
///
/// The [`payload`](Self::payload) is opaque bytes — the chat transport does
/// not interpret it. Lobby code AEAD-wraps room messages; raw lobby control
/// frames are JSON-encoded `LobbyMessage` enums.
#[derive(Debug, Clone)]
pub struct ChatFrame {
    /// Channel the frame was published on.
    pub channel_id: u64,
    /// Identity of the publishing peer. For in-memory transport this is the
    /// caller-chosen [`PeerInfo`]; for cryptographic transports it is the
    /// public key the transport authenticated the frame against.
    pub sender: PeerInfo,
    /// Opaque payload.
    pub payload: Bytes,
}

/// A peer participating on the chat transport.
///
/// The [`pubkey`](Self::pubkey) field is opaque so the trait stays agnostic
/// to the underlying cryptosystem (ed25519, secp256k1, x25519 wraps, etc.).
/// Lobby code parses it according to the transport in use.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct PeerInfo {
    /// Transport-specific identity bytes. Lobby code does not interpret;
    /// it passes the bytes through to AgentCard / on-chain identity lookups.
    pub pubkey: Vec<u8>,
    /// Optional human-readable address (multiaddr, URL, label).
    pub addr: Option<String>,
}

/// Error type returned by [`ChatTransport`] methods.
#[derive(Debug, thiserror::Error)]
pub enum ChatError {
    /// Caller tried to publish or subscribe to a channel that has not been
    /// registered (only relevant to transports that require pre-declaration).
    #[error("channel {0} is not registered")]
    ChannelNotRegistered(u64),
    /// Transport has been shut down and no longer accepts traffic.
    #[error("transport closed")]
    Closed,
    /// Underlying I/O failure (filesystem, network, etc.).
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    /// Implementation-specific error wrapped as a string for forward
    /// compatibility.
    #[error("transport error: {0}")]
    Other(String),
}

/// Result alias used throughout this crate.
pub type ChatResult<T> = Result<T, ChatError>;

/// A pinned, boxed stream of [`ChatFrame`]s. Returned by [`ChatTransport::subscribe`].
pub type FrameStream = Pin<Box<dyn Stream<Item = ChatFrame> + Send>>;

/// Pluggable chat transport.
///
/// Concrete impls live in adapter crates (`daeji-chat`, `iroh-chat`,
/// `local-chat-fs`, …); this crate ships [`InMemoryChat`] for tests and
/// local-first demos.
///
/// # Usage shape
///
/// ```ignore
/// # use chat_transport::{ChatTransport, InMemoryChat, PeerInfo};
/// # use bytes::Bytes;
/// # use futures::StreamExt;
/// # async fn run() -> Result<(), Box<dyn std::error::Error>> {
/// let me = PeerInfo { pubkey: b"alice".to_vec(), addr: None };
/// let transport = InMemoryChat::new(me);
/// transport.register_channels(&[42]).await?;
/// let mut sub = transport.subscribe(42);
/// transport.publish(42, Bytes::from_static(b"hello")).await?;
/// let frame = sub.next().await.expect("frame");
/// assert_eq!(frame.payload, &b"hello"[..]);
/// # Ok(()) }
/// ```
#[async_trait]
pub trait ChatTransport: Send + Sync {
    /// Subscribe to a channel. The returned stream yields each frame
    /// published to `channel_id` after the subscription was created.
    ///
    /// Implementors decide what happens on consumer lag — `InMemoryChat`
    /// drops the oldest frame; commonware-p2p signals a "lagged" notice.
    fn subscribe(&self, channel_id: u64) -> FrameStream;

    /// Publish a frame to a channel.
    ///
    /// Returns [`ChatError::ChannelNotRegistered`] if the transport requires
    /// pre-declaration and the channel has not been registered.
    async fn publish(&self, channel_id: u64, payload: Bytes) -> ChatResult<()>;

    /// Pre-register channels at startup. No-op for transports that don't
    /// require pre-declaration.
    ///
    /// Commonware-p2p requires every channel be declared before
    /// `network.start()`; the lobby + 64-slot-pool layout calls this once
    /// at agent boot with the canonical 65 channel ids.
    async fn register_channels(&self, channels: &[u64]) -> ChatResult<()>;

    /// Snapshot of the current peer set known to this transport.
    fn peers(&self) -> Vec<PeerInfo>;

    /// This transport's local identity. Frames published by us carry this
    /// in [`ChatFrame::sender`].
    fn local_identity(&self) -> PeerInfo;
}
