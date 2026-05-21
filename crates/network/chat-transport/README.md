# chat-transport

Chat-transport abstraction. Decouples lobby + marketplace + agent-runtime code from any specific chat implementation.

## Why

The 2026-05-18 architectural split puts the chat module owner (Jacob) on a different axis from the lobby/marketplace owner (Jae). The chat may be commonware-p2p, libp2p, iroh, or something fresh — but lobby code must be agnostic.

This crate ships the `ChatTransport` trait + an `InMemoryChat` impl. Concrete network transports (commonware-p2p, libp2p, iroh) implement the same trait in their own adapter crates and slot in behind a flag.

## Shape

```rust
#[async_trait]
pub trait ChatTransport: Send + Sync {
    fn subscribe(&self, channel_id: u64) -> FrameStream;
    async fn publish(&self, channel_id: u64, payload: Bytes) -> ChatResult<()>;
    async fn register_channels(&self, channels: &[u64]) -> ChatResult<()>;
    fn peers(&self) -> Vec<PeerInfo>;
    fn local_identity(&self) -> PeerInfo;
}
```

Channels are flat `u64`s. Frames are opaque bytes. Lobby code AEAD-wraps room messages above this layer; transport just moves bytes.

## InMemoryChat

In-process, tokio-broadcast-backed. Use for tests and the off-chain + local-first symphony demo (multiple agents in one runtime; no network).

For multi-process locality without network, a follow-up `local-chat-fs` adapter writes the same frame shape under `~/.nunchi/local-chat/`.

## Adapters (one impl per transport, each in its own crate)

| Adapter | Crate | Status |
|---------|-------|--------|
| In-memory | `chat-transport` (this crate) | shipped |
| Filesystem mirror | `local-chat-fs` | follow-up |
| commonware-p2p | `daeji-chat` (existing) | not yet rewired to this trait |
| libp2p | TBD | exploration in `Nunchi-trade/daeji#46` |
| iroh | TBD | exploration in `Nunchi-trade/daeji#45` (closed) |

## Tests

```bash
cargo test -p chat-transport
```

Covers publish/subscribe roundtrip, channel isolation, multi-subscriber fan-out, connected-set abstraction, and idempotent channel registration.
