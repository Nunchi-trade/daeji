# PRD: Nunchi Chat libp2p Rebuild (embedded validator subsystem)

## Status

- **Replaces:** closed PR #52 — `feat(nunchi-chat): add Iroh submesh and identity signing`,
  closed 2026-05-18. Close reason, verbatim: *"used both commonware-p2p and iroh, plus did
  not properly authenticate."*
- **Supersedes:** `docs/nunchi-chat-iroh-submesh-prd.md` (existed only on the #52 branch);
  the 2026-04-29 "commonware-p2p is the cooperative chat bus / two transports coexist"
  decision; the `agent-chat` canonical-plan §13/§14 transport assumptions.
- **Decision provenance:** the 2026-05-18 Jacob Gadikian ↔ Jae call — close #52, rebuild
  `nunchi-chat` from scratch on **libp2p** (chosen over Iroh: Iroh is CLI-only, libp2p has a
  full browser/JS implementation and a modular network stack).
- **State:** draft for review. This PR adds **only this document** — no implementation
  code. It is the spec to build against.

## Summary

Rebuild `nunchi-chat` as a **libp2p** agent-coordination subsystem, **embedded inside the
`kora` validator node** behind a cargo feature and an `--enable-chat` flag, isolated from
consensus, with EVM-identity-authenticated room admission and direct on-chain transcription
of chat outcomes through the node's in-process mempool.

One wire protocol (`/nunchi-chat/1.0.0`) serves two clients: the embedded node subsystem
(Rust, this repo) and a browser client (js-libp2p, built separately). libp2p gossipsub is
the chat transport; `commonware-p2p` remains the **consensus** transport only.

This is a from-scratch rebuild. The transport-agnostic protocol logic from PR #52 — EVM
identity verification, room-id derivation, AEAD, the message/lobby enums, the chain-event
watcher — is ported. The commonware-p2p networking layer, the 64-slot channel pool, and the
orphaned Iroh types are discarded.

### Architecture-reversal notice

PR #52's PRD, the `nunchi-chat` README, and the `agent-chat` canonical-plan all mandate
**validator isolation** — chat as a separate P2P plane, validators carrying no chat
traffic, chat→chain via external RPC. **This PRD deliberately reverses that.** The chat
subsystem runs *inside* the validator process. Rationale: the 2026-05-18 call identified
that transcribing chat outcomes onto the chain "might require incorporating libp2p into the
validator node"; embedding removes the external-RPC hop and the separate-process
operational surface. The cost — a second P2P stack in the validator — is contained by
feature-gating, opt-in activation, hard resource bounds, supervised isolation, and zero
shared mutable state with consensus beyond an append-only mempool handle. See
*Architecture → Embedded subsystem* and *Risks*.

## Problem

PR #52 (+5,147 / −42, 23 files) failed for three reasons:

1. **Dual transport.** The crate carried a `commonware-p2p` networking layer (`service.rs`)
   *and* a set of Iroh transport types (`transport.rs`: `TransportKind::Iroh`,
   `IrohTopicId`, `IrohRoomMembership`). The Iroh types were never wired — `iroh` /
   `iroh-gossip` were declared as a default cargo feature but never added as dependencies.
   The result: one half-built transport plus one orphaned spec masquerading as code.
2. **Authentication never gated the transport.** `identity.rs` contained complete EIP-191
   verification (`ChatJoinEnvelope`, `SignedRoomMessage`, signer recovery, freshness,
   membership) — and it was correct. But the transport (`service.rs`, commonware-p2p)
   admitted peers purely by ed25519 transport key from a registry file via `oracle.track`.
   The EVM-identity layer and the transport-admission layer were disconnected universes:
   `service.rs` never called `identity.rs`. A peer's claimed EVM identity was never bound
   to the key it actually spoke from. That is what "did not properly authenticate" means.
3. **Static channels.** commonware-p2p requires every channel to be `register()`-ed before
   `network.start()`. PR #52 worked around this with a fixed pool of 1 lobby + 64 slot
   channels and a hash-to-slot map; `chain.rs` itself logs that `JobAwarded` auto-join is
   "NOT YET IMPLEMENTED ... Q-Open-6" *because* of this constraint. Unrelated jobs collided
   on slots; AEAD silently absorbed the collisions.

Separately, the multi-agent demo is blocked on a working message bus and cannot wait for
this rebuild to land (see *Discord Interim Demo Stopgap*).

## Goals

- A single chat transport — **libp2p** — that is browser-reachable.
- EVM-identity-bound peer and room admission that **actually gates** the transport — fixes
  failure (2).
- **Dynamic per-job topics** — no fixed channel pool, no slot collisions — fixes failure (3).
- The chat subsystem **embedded in the `kora` validator node**, but isolated: feature-gated,
  opt-in, kill-switchable, supervised, resource-bounded, no shared mutable state with
  consensus.
- **On-chain transcription** of chat outcomes via the node's in-process mempool — no
  external RPC hop, no separate process.
- Reuse the transport-agnostic protocol logic from PR #52 verbatim where it is correct.
- Freeze a versioned wire protocol so a js-libp2p browser client can interoperate.

## Non-Goals

- Not replacing consensus networking — `commonware-p2p` stays the consensus transport.
- Not making the validator a public relay or a peer in arbitrary chat meshes; the
  relay-server role is config-gated, default OFF.
- Not adding a chain-side `ChatTx` opcode or precompile — transcription uses ordinary
  signed EVM transactions.
- Not the per-job-type modules (ISFR / swarm / mining-bounty specifics) beyond the generic
  room — those are follow-ups.
- Not the browser-client implementation — this PRD freezes the wire contract so it *can* be
  built (separately, likely in `nunchi-cli` / `frontend-integration`).
- The Discord stopgap is throwaway scaffolding, not a bridged transport.

## Users

- Agent operators running `kora --enable-chat`.
- Browser users coordinating via a js-libp2p client.
- Requesters and agents coordinating per-job execution rooms.
- Chain engineers reviewing the consensus ↔ chat isolation boundary.

## Architecture

### Network planes

Two P2P stacks run inside one `kora` process:

```
            kora validator process
  ┌───────────────────────────────────────────────┐
  │  consensus plane          chat plane           │
  │  commonware-p2p           libp2p Swarm         │
  │  authenticated::discovery gossipsub + …        │
  │  block prod / finality    lobby + per-job rooms│
  │        │                        │             │
  │        │                        ▼             │
  │        │                  transcribe module    │
  │        │                        │             │
  │        ▼                        ▼             │
  │   simplex engine ◄──── LedgerService (mempool) │
  │                         (append-only handle)   │
  └───────────────────────────────────────────────┘
        agents / browsers  ◄─libp2p─►  chat plane
```

The two planes share the process and the tokio runtime. They share **no** networking
state — no peer set, no ports, no channels. The only object crossing the boundary is the
`LedgerService` mempool handle: append-only, already concurrency-safe, already shared with
the RPC server.

### libp2p transport

`rust-libp2p` (crate `libp2p`, from crates.io). One `#[derive(NetworkBehaviour)]` struct,
`ChatBehaviour`, composing:

- **`gossipsub`** — the core. One mesh per topic: a well-known lobby topic plus one dynamic
  topic per active job. `MessageAuthenticity::Signed` (a libp2p-key signature on every
  message; the EVM signature is a separate inner layer). `ValidationMode::Strict`. A custom
  `message_id_fn` over `(room_id, sender, nonce)`. `max_transmit_size` 64 KiB.
- **`identify`** — peer-metadata exchange; advertises the `/nunchi-chat/1.0.0` protocol and
  observed addresses.
- **`relay`** (circuit-relay-v2) **client** — a NAT'd browser or agent stays reachable via
  a relay. The relay *server* role is a config toggle, default OFF — a validator is not a
  public relay by default.
- **`dcutr`** — direct-connection upgrade (hole punching) through a relay.
- **`autonat`** — NAT-status detection so a node knows whether it needs a relay.
- **`ping`** — liveness / keepalive.
- **`kad`** (Kademlia) — included but **feature-gated, default OFF**. The embedded-validator
  default is an explicit `bootnodes` list, not a global DHT: unbounded peer discovery is
  exactly the exposure to keep off a validator. `kad` exists so a future open-membership
  chat network can opt in.

Transport stack: `tcp` + `quic` (quic-v1) + `websocket` + `webtransport`, wrapped in `dns`,
with `noise` encryption and `yamux` stream multiplexing (QUIC carries its own). `websocket`
/ `wss` is the guaranteed browser-reachable transport; `webtransport` is the preferred
modern browser path — its server-side maturity in the pinned `rust-libp2p` release is
verified at implementation time (see *Open Questions*).

**Protocol id `/nunchi-chat/1.0.0`** — semver; the `1.x` line is wire-stable, a breaking
change bumps major. The rust node and the js-libp2p client must advertise the same protocol
id and identical gossipsub parameters or they will not form a mesh.

The libp2p node key is **Ed25519**; the `PeerId` derives from it. This is the *same*
ed25519 key an agent already publishes as `transport.pubkey` in its on-chain status card —
which is what makes the identity binding below clean.

### Browser client (js-libp2p)

Out of scope to implement here; this PRD freezes the contract the rust side must honour:
the same `/nunchi-chat/1.0.0` protocol id; gossipsub via `@chainsafe/libp2p-gossipsub` with
matching parameters; `@libp2p/websockets` and/or `@libp2p/webtransport` transports;
circuit-relay-v2 + dcutr for NAT'd browsers. At least one rust node — a relay/bootstrap,
not necessarily a validator — must listen on `wss`. The EVM identity layer is
transport-agnostic JSON, so the browser signs the join/message envelopes with the user's
wallet: a session key, an embedded wallet, or a passkey/WebAuthn key — never a pasted seed
phrase.

### Embedded subsystem

The crate `crates/network/nunchi-chat/` is a library (auto-included by the workspace
`crates/network/*` glob) plus a thin standalone client binary (below).

`bin/kora` depends on `nunchi-chat` **only behind a cargo feature**. The hook point is
`ProductionRunner::run()` in `crates/node/runner/src/runner.rs`, immediately after the
consensus engine starts and before the function returns:

```rust
engine.start(transport.simplex.votes, transport.simplex.certs, transport.simplex.resolver);

#[cfg(feature = "chat")]
if config.chat.enabled {
    nunchi_chat::spawn_embedded(config.chat.clone(), ledger.clone(), self.chain_id);
}

info!("Validator started successfully");
Ok(ledger)
```

`ledger` (a `LedgerService`) is already in scope — it is the handle the function returns.
Cloning it into the chat subsystem is the entire transcription wiring; no new plumbing.
The subsystem is **spawned detached** — it can never block consensus startup or shutdown,
and `run()` returns `Ok(ledger)` exactly as today.

`spawn_embedded` builds one `libp2p::Swarm<ChatBehaviour>` and runs one `select!` event
loop on a spawned task. It touches neither the `commonware-p2p` transport nor the consensus
`oracle` / peer set. Its only inputs are `ChatConfig`, the `LedgerService` clone, and
`chain_id`.

Crate modules: `swarm` (builds the `Swarm` + transport + `ChatBehaviour`, owns the event
loop), `rooms` (active-job room table; dynamic topic subscribe/unsubscribe), `lobby`,
`identity` (EVM EIP-191 verification + PeerId binding), `room` (room-id + AEAD), `messages`,
`card`, `chain` (alloy event watcher), `transcribe` (chat outcome → signed `Tx` → mempool),
`supervisor` (backoff + kill switch), `config`.

A thin **standalone binary** `nunchi-chat` is also kept (ported from PR #52's
`src/bin/nunchi-chat.rs`): the browser-less agent client and the local multi-node test
harness. Embedded and standalone share one swarm/rooms/identity core and differ only in the
transcription sink — embedded submits to `LedgerService`, standalone submits via RPC.

### Isolation & resource bounds

The invariant: **a misbehaving or flooded chat plane degrades chat only — never
consensus.** Enforced by:

- A separate listen-port range, distinct from the consensus `NetworkConfig.listen_addr`;
  chat startup validates no port overlap and fails *chat-only* (never the node) on conflict.
- libp2p `ConnectionLimits` — max established (default 256), max pending in/out.
- gossipsub `max_transmit_size` 64 KiB; conservative `mesh_n` / `mesh_n_low` / `mesh_n_high`;
  bounded duplicate cache.
- A cap on concurrent active rooms (default 128); subscribing beyond the cap is refused and
  logged.
- A per-topic inbound message-rate limit plus a global inbound byte-rate cap (`governor`,
  already a workspace dependency).
- Bounded channels between the swarm task and any worker tasks — no unbounded queues.
- Supervision: a `supervisor` wrapper restarts a panicked swarm task with exponential
  backoff and a panic-rate ceiling, and re-checks the kill switch on every respawn.

### Identity & authentication

Three layers of identity, kept explicitly distinct:

- **Chain identity** (who you are) — the EVM address in `AgentRegistry` + the ERC-8004
  `passportId` + `passportHash`. Authoritative.
- **Transport identity** (how bytes route) — the libp2p `PeerId`, derived from the agent's
  Ed25519 transport key.
- **The binding** (proof the two are the same principal) — the agent's on-chain status card
  (`StatusCard`, verified by `keccak256(body) == passportHash`) publishes `transport.pubkey`
  (ed25519). The chain commits to the transport key; the `PeerId` is derived from that exact
  key. So the trust chain is: chain → card (keccak-verified) → ed25519 key → `PeerId`. No
  new signature scheme is needed for the binding — it falls out of reusing the agent's
  ed25519 transport key as the libp2p identity key.

**Join handshake — the PR #52 fix.** To join a room a peer publishes a `ChatJoinEnvelope` —
`{ agent_address, passport_id, topic, nonce, ts_ms, transport_pubkey_hex, signature }`, an
EIP-191 personal-sign over `"nunchi-chat.join\n{topic}\n{nonce}\n{ts_ms}"`. The receiver
runs `verify_room_join`: recover the EVM signer, require it equals `agent_address`, check
`ts_ms` freshness, check `topic` matches the room, check the recovered identity is an
expected member. **Then the check #52 lacked:** verify the `PeerId` the gossipsub message
actually arrived from equals `PeerId::from(ed25519(transport_pubkey_hex))`. A valid EVM
signature presented from a peer speaking on a *different* `PeerId` than its card-committed
key is rejected. This is the line that binds the EVM identity to the transport.

**Room admission.** A libp2p topic is discoverable by topic id, so topic membership is not
the security boundary. Two enforcement layers:

- *gossipsub explicit validation* — a room message from a `PeerId` outside the room's
  allowed set is reported `MessageAcceptance::Reject` (also penalized by gossipsub scoring)
  and not propagated.
- *application layer* — every room message is a `SignedRoomMessage` carrying an EIP-191
  per-message signature over `(domain, room_id, agent_address, transport_pubkey, nonce,
  ts_ms, payload_hash)`; `ReceivedRoomMessage::accept_or_drop` enforces
  `MessageSignaturePolicy`. Under `Required` (default) unsigned or invalid messages are
  silently dropped.
- *replay* — a per-`(room, sender)` seen-nonce cache rejects exact replays inside the
  freshness window. PR #52 checked freshness but not nonce-uniqueness; this is a hardening.

`expected_members` for a room is resolved from chain: `JobAwarded.winners` → `AgentRegistry`
lookups → each winner's card → `(agent_address, passport_id, ed25519 key)` → `PeerId`. Only
those `PeerId`s are admitted to the room mesh and accepted as room senders.

`MessageSignaturePolicy::LocalTestnetDisabled` — the unsigned-allowed carve-out — is
restricted to local/private builds, ideally compiled out of release builds behind a feature
gate so a public binary cannot select it.

### Rooms & messaging

- **Dynamic per-job topics.** One lobby topic plus one gossipsub topic per active job. On
  `JobAnnounce` / `JobAwarded` the subsystem `subscribe`s to the room topic; on
  `JobConcluded` it `unsubscribe`s. The PR #52 64-slot pool is removed — it existed only
  because commonware-p2p needs channels declared before `network.start()`; libp2p gossipsub
  subscribes dynamically.
- **Room id** (unchanged — cross-language parity preserved): `room_id_for_chain_job(job_id)`
  = `keccak256("NUNCHI_ROOM_V1" || uint256_be(job_id))`, matching the on-chain
  `MultiAgentMarket.computeRoomId(uint256)`. The room's gossipsub topic string is
  `"/nunchi-chat/1.0.0/room/" + hex(room_id)`; the lobby topic is
  `"/nunchi-chat/1.0.0/lobby"`.
- **AEAD** (unchanged): `ChaCha20Poly1305`, wire layout `nonce(12) || ciphertext`, the
  `room_id` bound as additional authenticated data. A gossipsub room-message payload is the
  AEAD ciphertext of the JSON-serialized `SignedRoomMessage` — a non-member without the room
  key cannot even see the signature, let alone the content.
- **Room-key wrap — implemented properly.** The coordinator generates a random 32-byte room
  key and wraps it per participant via **X25519 ECDH**: the participant's Ed25519 transport
  key is mapped to X25519 (standard birational map), ECDH against an ephemeral key, HKDF to
  a wrapping key, AEAD-seal the room key. `RoomKeyWrap.ciphertext_hex` carries ephemeral
  pubkey + nonce + sealed key. PR #52 shipped *plaintext* wraps; the rebuild ships ECDH
  (phase W6; plaintext stays local-private-only until then).
- **Messages** (ported): `RoomMessage` — `Hello` / `Status` / `PartialResult` / `Vote` /
  `Final`. `LobbyMessage` — `JobAnnounce` / `RoomJoined` / `JobConcluded` / `MiningClaim`,
  with the `slot_index` field removed (no slots). The lobby is plaintext JSON; confidential
  bits travel only inside `RoomKeyWrap`.

### On-chain transcription

A chat *outcome* is a room reaching a terminal state — e.g. the awarded agent posts a
`RoomMessage::Final`, or a quorum of `Vote`s agree. The `transcribe` module watches verified
room messages, detects the terminal condition, and submits the resulting chain action
through the **in-process mempool**:

```
chat outcome → signed EVM transaction bytes → Tx::new(bytes)
            → TransactionValidator::validate(...) → LedgerService::submit_tx(tx)
```

This is the exact path the RPC server already uses (`runner.rs`, the `TxSubmitCallback`).
The subsystem holds a `LedgerService` clone — no RPC socket, no separate process.

**Who signs the transcription tx — recommended: the agent.** The chat outcome itself
carries an agent-signed EVM transaction (or the signed material the agent's own client
submits); the embedded subsystem only *relays* bytes into the local mempool. The validator
never holds application-signing authority and never authors application transactions. The
rejected alternative — a node-held signing key — would make the validator a signer of
application state and is not adopted.

**Isolation.** Transcription only ever *adds* to the mempool. It never touches consensus
voting, block production, or the simplex engine. A transcription tx is an ordinary tx the
validator set includes (or not) via normal block production. **Chat can propose chain
state; it cannot finalize it.**

The `chain` watcher (an alloy WS subscriber to `AgentRegistered` and `JobAwarded`) runs as
a bounded background task. Embedded, it connects over loopback `ws://` to the node's own
RPC endpoint; reading node-local state directly is a later optimization, not v1.

## Configuration

`ChatConfig` becomes a field on `NodeConfig` (`crates/node/config/src/node.rs`):
`#[serde(default)] pub chat: ChatConfig`. A `[chat]` TOML table configures it; its absence
means chat is disabled. A new `chat.rs` config module sits alongside `network.rs`.

```toml
[chat]
enabled                  = false
listen_addrs             = ["/ip4/0.0.0.0/tcp/9600", "/ip4/0.0.0.0/udp/9600/quic-v1"]
bootnodes                = []      # ["/dns4/<host>/tcp/9600/p2p/<peer-id>", ...]
relay_addrs              = []
enable_relay_server      = false
enable_kademlia          = false
max_connections          = 256
max_rooms                = 128
message_signature_policy = "required"   # or "local-testnet-disabled" (private builds only)

[chat.chain]                # optional; enables the chain-event watcher + transcription
rpc_ws         = "ws://127.0.0.1:8545"
agent_registry = "0x..."    # contracts-core AgentRegistry — address is per-deployment
market         = "0x..."    # contracts-core MultiAgentMarket — address is per-deployment
```

- **Cargo feature `chat`** on `bin/kora` — `nunchi-chat` is an optional dependency. The
  feature is **not** in `default` initially (libp2p stays out of default builds until the
  subsystem is proven), then flipped to default-on once stable (criteria in *Open
  Questions*). CI note: `ci.yml`'s `build` job runs `cargo build --all-targets` *without*
  `--all-features`, so it will not compile the chat path while the feature is non-default —
  W1 adds `--features chat` to the build job (or a dedicated matrix entry).
- **`--enable-chat` CLI flag** on `kora` — runtime opt-in even when the feature is compiled
  in; flips `config.chat.enabled`. With the feature off, the flag errors clearly.
- **Kill switch `NUNCHI_CHAT_DISABLED`** (env var) — checked at startup before the swarm is
  built and re-checked by the supervisor on each respawn, so an operator can disable a
  running chat plane without restarting `kora`.

## Migration

PR #52's modules split cleanly into transport-agnostic logic (port) and
commonware-p2p-specific machinery (discard):

| PR #52 artifact | Disposition | Notes |
|---|---|---|
| `identity.rs` — EIP-191 verify, `ChatJoinEnvelope`, `SignedRoomMessage`, `MessageSignaturePolicy`, recover/freshness | **PORT** (≈verbatim) | Logic correct; was never wired to the transport. Add the PeerId-binding check + a nonce-replay cache. |
| `room.rs` AEAD — `encrypt`/`decrypt`, `room_id`, `room_id_for_chain_job` | **PORT** | Unchanged. Keep the Solidity cross-language parity test. |
| `room.rs` slot pool — `POOL_SIZE`, `slot_pool_base`, `slot_for_chain_job`, `channel_id_for_slot`, `channel_id_from_room`, `lobby_channel_id` | **DISCARD** | commonware-p2p workaround; gossipsub has dynamic topics. |
| `messages.rs` — `RoomMessage` | **PORT** | Unchanged. |
| `lobby.rs` — `LobbyMessage`, `RoomKeyWrap` | **PORT, EDIT** | Drop the `slot_index` fields. Implement real X25519 ECDH for `RoomKeyWrap` (was plaintext). |
| `card.rs` — `StatusCard`, `keccak256(body)==passportHash`, ed25519 pubkey decode | **PORT** | Root of the chain → PeerId trust chain. |
| `chain.rs` — alloy WS subscriber for `AgentRegistered` / `JobAwarded` | **PORT, EDIT** | Drop slot/channel-id derivation. `JobAwarded` auto-join now works (dynamic topics) — implement it instead of the "Q-Open-6 deferred" log. |
| `registry.rs` — `AgentRecord` (Seed/Chain), `Registry` file | **PORT, ADAPT** | The on-chain-derived peer set now feeds the libp2p PeerId allowlist instead of commonware `oracle.track`. |
| `service.rs` — commonware-p2p `discovery::Network`, lobby + 64 slot channels, registry poller | **DISCARD** | The whole commonware-p2p networking layer is replaced by the libp2p `Swarm`. |
| `transport.rs` — `TransportKind::Iroh`, `IrohTopicId`, `IrohRoomMembership` | **DISCARD types, PORT concepts** | Iroh-named types die. `IrohRoomMembership` logic (`contains_identity`, expected-members) becomes plain `RoomMembership`. |
| `supervisor.rs` — `NUNCHI_CHAT_DISABLED`, backoff, panic tracking | **PORT** | Wrap the libp2p event loop instead of the old `run_chat`. |
| `src/bin/nunchi-chat.rs` — standalone binary | **PORT, ADAPT** | Kept as the standalone / browser-less client; same swarm core, transcribes via RPC. |
| `commonware-p2p` / `commonware-runtime` deps in the chat crate | **DISCARD** | Chat no longer touches commonware networking or runtime. |
| `docs/nunchi-chat-iroh-submesh-prd.md` | **SUPERSEDE** | Replaced by this document. |

**Superseded prior decisions** (stated for the record):

- 2026-04-29 "commonware-p2p is the cooperative chat bus; two transports coexist" —
  superseded. Two P2P stacks still run in the process, but the chat one is now **libp2p**;
  commonware-p2p is consensus-only.
- `agent-chat` canonical-plan §13 (chat on the same authenticated commonware mesh as
  consensus) — superseded; chat runs its own libp2p `Swarm`.
- `agent-chat` canonical-plan §14 (lobby + 64-slot pool) — superseded by dynamic topics.

Nothing chat-related is currently merged to `main` — all of PR #10–#52 are closed — so this
is a clean rebuild, not an edit of a live crate.

## Implementation Plan

Seven phases. Each ends with a compile + test + demonstrate checkpoint before the next.

- **W1 — Crate skeleton + dependency clearance.** Create `crates/network/nunchi-chat/`;
  wire the workspace lints. Add `libp2p` (pinned, verified version) with the chosen feature
  set, plus `x25519-dalek` / `chacha20poly1305` / `k256` / `sha3` / `alloy` / `serde`. Run
  `cargo deny` and reconcile `deny.toml` (licenses + advisories) for the libp2p dependency
  tree. Add a `chat`-feature build to CI. Port the pure modules with no networking — `room`,
  `messages`, `card`, `identity` — and their unit tests. *Exit:* `cargo nextest run -p
  nunchi-chat` green; `cargo deny` green.
- **W2 — libp2p swarm core (lobby only).** Build `ChatBehaviour` (gossipsub + identify +
  ping + relay-client + dcutr + autonat) and the transport stack (tcp + quic + ws + dns,
  noise + yamux); run the event loop. Subscribe to the lobby topic; send/receive plaintext
  `LobbyMessage`. Derive the `PeerId` from the ed25519 transport key. *Exit:* a 2-node local
  lobby smoke test passes.
- **W3 — Dynamic per-job rooms + AEAD.** On `JobAnnounce`, derive the room topic from
  `room_id` and `subscribe`; on `JobConcluded`, `unsubscribe`. Room messages are
  AEAD-sealed `SignedRoomMessage`s. Build the room-membership table. *Exit:* a 3-node local
  test runs `Hello` → `Status` → `Final` over one encrypted dynamic topic.
- **W4 — Identity binding + authenticated admission (the core fix).** Wire
  `verify_room_join` into the join path; add the `PeerId`-binding check. Add the gossipsub
  per-message validation callback that `Reject`s non-member `PeerId`s. Enforce
  `MessageSignaturePolicy::Required` + the nonce-replay cache. Maintain the global
  active-agent `PeerId` allowlist fed by the registry. *Exit:* isolation tests — an
  uninvited node cannot get a room message accepted; a spoofed-`PeerId` join is rejected;
  unsigned messages are dropped under `Required`.
- **W5 — Embed in `kora`.** Add the `chat` cargo feature to `bin/kora`; make `nunchi-chat`
  an optional dep. Add `--enable-chat` to `cli.rs` and `[chat]` `ChatConfig` to
  `NodeConfig`. Call `nunchi_chat::spawn_embedded(...)` inside `ProductionRunner::run()`
  after `engine.start(...)`, with a `LedgerService` clone. Wrap the swarm loop in the
  supervisor; enforce all resource bounds. *Exit:* `kora --enable-chat` boots and runs
  consensus *and* a chat swarm; killing or flooding the chat plane leaves consensus healthy.
- **W6 — On-chain transcription + X25519 room-key wrap.** Build the `transcribe` module:
  detect a room terminal condition, build a signed EVM `Tx`, submit via
  `LedgerService::submit_tx` (agent-signed, node-relays). Replace the plaintext
  `RoomKeyWrap` with real X25519 ECDH. Embed the `chain` watcher so `JobAwarded` drives real
  auto-join. *Exit:* an end-to-end devnet run where a chat `Final` becomes a tx in the local
  mempool and lands in a block.
- **W7 — Browser interop + hardening.** Validate a js-libp2p client joining a room over
  `wss` / WebTransport and exchanging messages with rust nodes (the JS client itself may be
  a separate PR; this phase is the rust-side interop check + a documented `wss` listener
  config). Add gossipsub peer scoring; metrics (`prometheus-client`, already a workspace
  dep) for per-room peer count, message rate, relay usage, transcription count. Add
  `cargo-fuzz` targets for `identity` and AEAD. Land operator docs in `docs/`. *Exit:*
  browser↔node interop demonstrated; fuzz targets run clean; docs landed.

W1–W4 produce a working standalone libp2p chat **independent of `kora`** — the crate is
testable and reviewable before the validator embedding lands. The embedding (W5) and
transcription (W6) are the higher-risk, chain-adjacent phases and come after the transport
is proven. This sequencing directly serves the "land the P2P evaluation before committing
the chat↔chain integration" concern from the 2026-05-18 call.

## Test Strategy

- **Unit** (per module): `room` — AEAD round-trip, wrong-key/wrong-room rejection, the
  Solidity room-id parity vector (ported verbatim); `identity` — EIP-191 recovery,
  signer-mismatch, freshness/replay window, membership, the `accept_or_drop` policy matrix,
  plus new nonce-replay and PeerId-binding cases; `card` — schema parse, `passportHash`
  verification, ed25519 pubkey hex/base64 decode; `messages`/`lobby` — serde round-trips,
  unknown-variant rejection; `transcribe` — terminal-condition detection, `Final` → `Tx`
  byte-shape.
- **Integration**: 2-node lobby exchange; 3-node single-room flow; dynamic-topic lifecycle
  (subscribe on `JobAnnounce`, unsubscribe on `JobConcluded`, no traffic after leave);
  X25519 room-key wrap/unwrap across two parties.
- **Multi-node**: a 3–5 node swarm with multiple concurrent rooms — confirm room isolation
  (a node in room A never decrypts room B); a NAT/relay path — a node with no public
  address reaches a peer via circuit-relay-v2 then dcutr-upgrades to direct — exercised
  **off a single host** so NAT behaviour is real.
- **Browser interop**: a js-libp2p client (headless browser or a Node js-libp2p harness)
  joins a rust-hosted room over `wss`, exchanges a signed + encrypted message, and is
  accepted by the rust side and vice versa. Protocol-id and gossipsub-parameter parity is
  the property under test.
- **Embedded-subsystem isolation** (the most important new category): boot `kora
  --enable-chat` and assert consensus produces blocks normally; flood the chat plane (many
  rooms, large messages, rapid join/leave) and assert block cadence and finality are
  unaffected; kill/panic the chat task and assert the supervisor backs off while consensus
  is untouched; assert that with the `chat` feature OFF the `kora` build has zero libp2p in
  its dependency graph; a port-conflict test (overlapping ports fail chat-only, loudly,
  without killing the node); a shutdown-ordering test (a slow chat shutdown cannot hang
  node exit).
- **Fuzzing** (`cargo-fuzz` targets): `identity` — fuzz `ChatJoinEnvelope` /
  `SignedRoomMessage` deserialization and signature recovery (malformed signatures,
  off-curve points, length edge cases); `room` AEAD — fuzz `decrypt` against arbitrary
  bytes (must never panic, always `None` on bad input); `RoomKeyWrap` unwrap; the end-to-end
  AEAD-sealed `SignedRoomMessage` open path.
- **CI**: the existing jobs (`build`, `test --all-features`, `fmt`, `clippy`, `deny`) stay
  green; W1 adds chat-feature build coverage. Multi-node and browser-interop tests are
  likely too heavy for the default `pull_request` run — propose a separate workflow or a
  nightly job.

## Discord Interim Demo Stopgap

The 2026-05-18 call also decided to stand up a Discord server as a temporary message bus so
the two demo "open-call agents" can collaborate off-chain *now*, without waiting for libp2p.

This is **throwaway scaffolding, not a bridge.** Explicitly: the Discord stopgap will not be
wired into the libp2p chat plane, there is no Discord↔libp2p bridge on the roadmap, and the
Discord path is retired once W2/W3 give two nodes a working libp2p room. It demonstrates the
agent-collaboration *UX*, not the production transport — and that distinction should stay
explicit internally and with investors.

It belongs in this PRD only because it sets the roadmap's external pressure: the demo is
unblocked by Discord, so the libp2p rebuild does **not** have to be rushed to a demo date —
it can follow W1→W7 with proper testing. Discord is the reason the rebuild can be done
correctly rather than hastily.

The one acceptable point of contact is a *schema convention*, not code: the demo's Discord
messages can reuse the `RoomMessage` shape (`Hello`/`Status`/`PartialResult`/`Vote`/`Final`)
as their logical schema so the demo's semantics match the eventual libp2p semantics. The
Discord bot lives outside `daeji` entirely (in the agent-tooling repo) and is out of scope
for the `nunchi-chat` crate.

## Rollout

1. Merge this PRD.
2. Build W1–W4 — the crate, standalone and `kora`-independent.
3. Land the crate behind the `chat` feature, off by default.
4. Build W5–W6 — embed in `kora`, devnet smoke with `--enable-chat`.
5. Build W7 — browser interop.
6. Enable `--enable-chat` on a controlled testnet.
7. Security review with chain engineering — covering the embedded-subsystem attack surface
   — before any mainnet exposure; only then consider flipping the `chat` feature default.

## Risks

Running two P2P stacks in one process is the central risk. Each item below has a stated
mitigation; the embedded-subsystem isolation tests are the proof.

- **Port conflicts** — consensus binds `NetworkConfig.listen_addr`, chat binds
  `ChatConfig.listen_addrs`; they must differ. *Mitigation:* chat defaults to a separate
  port range and validates non-overlap at startup, failing chat-only on conflict.
- **Runtime / executor contention** — the node runs on `commonware_runtime::tokio` (a
  commonware wrapper over tokio); libp2p's `tokio` feature expects a standard tokio runtime.
  *This is the single biggest unknown* — see *Open Questions*. *Mitigation:* spike whether
  the swarm can be driven on the existing runtime; fall back to a dedicated, bounded
  `tokio::runtime::Runtime` for chat if scheduler interference is observed.
- **CPU / scheduler starvation** — a chat flood could starve consensus tasks on a shared
  pool. *Mitigation:* resource bounds + the dedicated-runtime fallback; the flood isolation
  test is the proof.
- **Memory pressure** — two networking stacks' buffers and caches. *Mitigation:* bounded
  room table, dedup cache, connection limits, gossipsub `max_transmit_size`.
- **Shutdown ordering** — both stacks must stop without deadlock. *Mitigation:* the chat
  swarm is a detached, independently cancellable task; its drain has a timeout and cannot
  block node exit.
- **Widened validator attack surface** — embedding network-facing code in the validator is
  the cost of this architecture. *Mitigation:* feature-gated (a build can exclude chat
  entirely), `--enable-chat` opt-in, the kill switch, bounded resources, supervision, no
  shared state with consensus beyond the append-only mempool handle, strict message
  validation, and no DHT by default.
- **libp2p semver churn** — pre-1.0 `rust-libp2p` minor releases move `NetworkBehaviour`
  APIs; an upgrade is a real maintenance event. *Mitigation:* pin exactly; isolate libp2p
  types behind the crate's module boundary so an upgrade touches one crate.
- **Dependency-tree growth** — libp2p pulls a large tree; compile time and binary size
  grow, and licenses/advisories need reconciling against `deny.toml`. *Mitigation:* W1
  treats `cargo deny` reconciliation as explicit scope.

## Open Questions

- **Exact `libp2p` crate version** — pin the latest stable `rust-libp2p` release verified on
  crates.io at implementation time; record the exact version and resolved transitive set in
  `Cargo.lock`. Not asserted here, to avoid a stale pin.
- **WebTransport server maturity** in the pinned `rust-libp2p` release — verify; `websocket`
  / `wss` is the guaranteed browser fallback if WebTransport is not yet stable.
- **Transcription target contract + selector** — which `contracts-core` contract and
  function a transcription tx calls is **not specified here**; `contracts-core` is a
  separate repo. To be confirmed with the chain/contracts team — do not invent an address
  or selector.
- **Runtime coexistence** — can the libp2p `Swarm` be driven on the commonware-tokio
  runtime, or does chat need its own bounded `tokio::runtime::Runtime`? Resolve with a W2/W5
  spike.
- **`chat` feature default** — ships off; the criteria to flip it default-on (stability
  bar, security review sign-off) to be agreed.
- **Relay-server role** — should designated infra nodes (not validators) host libp2p relays,
  and where are their multiaddrs published — config, or the on-chain card?
- **Lobby sharding** — a single global lobby for v1; whether to later shard by
  network/market/job-class is carried forward from PR #52's open questions.
- **Embedding hook** — `ProductionRunner::run` is the chosen hook; whether the legacy
  no-subcommand path (`LegacyNodeService`) also needs chat is to be confirmed.

## Appendix

### A. Crate dependencies

New to the workspace: `libp2p` (with `gossipsub`, `identify`, `kad`, `relay`, `dcutr`,
`autonat`, `ping`, `noise`, `yamux`, `tcp`, `quic`, `websocket`, `webtransport`, `dns`,
`macros`, `tokio`); `x25519-dalek` and `curve25519-dalek` (room-key ECDH). Already used by
PR #52 / present in the workspace and reused as-is: `chacha20poly1305`, `k256`, `sha3`,
`hex`, `base64`, `serde` / `serde_json`, `thiserror`, `alloy`, `reqwest`, `tracing`,
`futures`, `governor`, `prometheus-client`. Exact versions are pinned at implementation
time and recorded in `Cargo.lock`; new licenses/advisories are reconciled in `deny.toml`.

### B. Wire formats

- `RoomMessage` — JSON, `#[serde(tag = "type", rename_all = "snake_case")]`: `hello`,
  `status`, `partial_result`, `vote`, `final`.
- `LobbyMessage` — JSON, `#[serde(tag = "type", rename_all = "snake_case")]`:
  `job_announce`, `room_joined`, `job_concluded`, `mining_claim` (no `slot_index`).
- `ChatJoinEnvelope` signing string: `"nunchi-chat.join\n{topic}\n{nonce}\n{ts_ms}"`,
  EIP-191 personal-sign.
- `SignedRoomMessage` signing string:
  `"nunchi-chat.message\n{room_id}\n{agent_address}\n{transport_pubkey_hex}\n{nonce}\n{ts_ms}\n{payload_hash_hex}"`,
  EIP-191 personal-sign; `payload_hash_hex` = `keccak256(serde_json(RoomMessage))`.
- Room-message gossipsub payload: `ChaCha20Poly1305(nonce(12) || ciphertext)` of the
  JSON-serialized `SignedRoomMessage`, with `room_id` as AAD.

### C. Worked example — room-id derivation

For chain `job_id = 7`:

```
room_id = keccak256( b"NUNCHI_ROOM_V1" || uint256_be(7) )
        = 0x893074710278d32ed80a54d72a2def6f25b72729a4aeaf64e33a943274a0a833
gossipsub topic = "/nunchi-chat/1.0.0/room/893074710278d32ed80a54d72a2def6f25b72729a4aeaf64e33a943274a0a833"
```

This value is fixed by the existing cross-language parity test against
`MultiAgentMarket.computeRoomId(7)` and must not change across the rebuild.
