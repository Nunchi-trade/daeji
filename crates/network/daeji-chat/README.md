# daeji-chat

Symphony-class agent-coordination **protocol primitives + service entry point** for Daeji. Library-only crate; **no standalone binaries**. Designed to be **bolted into the existing `kora` chain binary** (not run as a separate process), so chat lives on the same authenticated commonware mesh as consensus.

This PR (#24) lands the **full layer**: the protocol primitives, the kora hookup (`--chat-config` / `--disable-chat`), and the supervised spawn — replacing the 5-PR stack (#11/#13/#14/#17/#19) with a single reviewable unit.

## Why a library + zero binaries

Per the canonical-plan §13 and explicit user direction: **chat is the first non-EVM component on Daeji.** Validators / followers run kora; when chat is enabled, chat handlers run as additional commonware channels alongside consensus / mempool / blocks **on the same authenticated mesh**.

A separate `daeji-chat` binary process would mean a parallel mesh, separate peer-set sync, separate identity story. We don't want any of that. So this crate is just the protocol — the kora binary calls into it.

## How chat works (end-to-end)

The chat layer is one **lobby channel** plus a **fixed pool of 64 slot channels**, all pre-registered at `network.start()`. Per-job state is populated dynamically.

```
on-chain                 lobby (1 channel)            slot channels (64, pool)
────────                 ──────────────────           ────────────────────────
JobAwarded ─────► JobAnnounce(job, slot, key)
                  ───────────────────────────► RoomMessage::Hello
                                                 (AEAD, room id as AAD)
                                                RoomMessage::Status
                                                RoomMessage::PartialResult
                                                RoomMessage::Vote
                                                RoomMessage::Final
                  JobConcluded(job, slot)  ◄── (slot freed, reusable)
```

1. **Job awarded on chain.** `MultiAgentMarket.JobAwarded(id, winners[], roomId)` fires. The chain watcher (`chain.rs`) logs it and verifies our derived `room_id == roomId` (cross-language parity with `keccak256("DAEJI_ROOM_V1" || job_id)`).
2. **Coordinator broadcasts `JobAnnounce` on the lobby.** Carries `job_id`, `slot_index`, the room key wrapped per-recipient (`RoomKeyWrap`), and the announced-at block. Lobby messages are plaintext JSON; only the room keys are wrapped.
3. **Awarded agents unwrap and join.** `service.rs` inserts `(job_id → ActiveJob)` into the in-memory map keyed off the slot index, then optionally broadcasts `RoomJoined` so the room sees liveness.
4. **In-room traffic is AEAD-encrypted.** ChaCha20Poly1305, room-id bound as AAD (so a packet from one room can't be replayed into another). Multiple jobs sharing a slot succeed by **try-decrypt against each active job's key** — first key that authenticates the tag routes the message; all-fail discards. POOL_SIZE=64 keeps the slot collision rate low without blowing up the channel registry.
5. **Coordinator (or any winner) sends `JobConcluded`.** Slot entry is removed and the slot is immediately reusable for new `JobAnnounce`s.

**Coordinator role varies by job type** (canonical-plan §11):

- **Symphony / reputation-gated / swarm-open** — requester broadcasts `JobAnnounce` after their `JobAwarded` tx confirms (they hold the bounty + selected the winners + can mint the room key).
- **Mining-bounty** (winner-takes-all) — claim-first model. Agents broadcast `MiningClaim` on the lobby; first valid claim settles. No central coordinator.
- **Public-broadcast** (ISFR / autoresearch) — no lobby; agents post estimates on well-known signal channels directly.
- **MEV race** — no lobby; latency budget doesn't allow it.

## Module map

| Module | Purpose |
|---|---|
| `card.rs` | `StatusCard` agent-card schema + `keccak256(body) == passportHash` verifier. Decodes ed25519 transport pubkeys (hex or base64). |
| `chain.rs` | alloy WS subscriber for `AgentRegistered` + `JobAwarded`. Verifies status-card → updates registry file (the runtime's 200ms poller picks it up and refreshes `oracle.track`). |
| `lobby.rs` | `LobbyMessage` enum (`JobAnnounce` / `RoomJoined` / `JobConcluded` / `MiningClaim`) + `RoomKeyWrap`. |
| `messages.rs` | `RoomMessage` enum (`Hello` / `Status` / `PartialResult` / `Vote` / `Final`) — wire format inside a room. |
| `registry.rs` | File-backed authorized peer set (`Seed`-shortcut and `Chain`-sourced records). 200ms poll → `oracle.track` epoch refresh. |
| `room.rs` | Deterministic room id (`keccak256("DAEJI_ROOM_V1" \|\| job_id)`), slot-id derivation matching on-chain `MultiAgentMarket.computeRoomId`, `POOL_SIZE = 64`, ChaCha20Poly1305 AEAD with room-id-as-AAD. Cross-language parity test against `cast keccak`. |
| `service.rs` | `run_chat(ctx, ChatConfig)` — pre-registers lobby + 64 slot channels, spawns lobby listener + 64 try-decrypt loops. |
| `supervisor.rs` | Supervised wrapper: exponential backoff (1s→60s), `PanicTracker`, `DAEJI_CHAT_DISABLED` runtime kill switch. |

## How this PR consolidates the 5 prior PRs

This PR (#24) **supersedes** the original 5-PR stack and is the single reviewable unit. After it lands, the originals close as superseded.

| Prior PR | Branch | What it added | Now in this PR |
|---|---|---|---|
| #11 | `chat-chain-events` | `chain.rs` + status-card verification | `crates/network/daeji-chat/src/chain.rs` + `card.rs` |
| #13 | `lobby+slot pool primitives` | `lobby.rs` + `room.rs` constants (`POOL_SIZE`, slot derivation) | `crates/network/daeji-chat/src/lobby.rs` + `room.rs` |
| #14 | `lobby+slot pool runtime` | `service.rs` refactor — pre-register lobby + 64 slot channels | `crates/network/daeji-chat/src/service.rs` |
| #17 | `supervisor primitives` | `supervisor.rs` — backoff + panic tracker | `crates/network/daeji-chat/src/supervisor.rs` |
| #19 | `supervised chat hookup` | `--chat-config` + `--disable-chat` CLI flags + supervised spawn in kora-service | `bin/kora/src/cli.rs` + `crates/node/service/` |

The behavior is identical to the 5-PR stack; the consolidation is purely structural (one diff to review, one merge to land).

## How to wire this on chain

The chat layer is paired with three on-chain contracts in [`Nunchi-trade/contracts-core`](https://github.com/Nunchi-trade/contracts-core):

- `AgentRegistry.sol` — agent identity + passport-hash binding.
- `MultiAgentMarket.sol` — job posting, awarding, and `roomId` emission.
- `ConsortiumValidator.sol` / `IResolverTarget.sol` — final-result settlement.

### 1. Deploy the contracts

`AgentRegistry` and `MultiAgentMarket` go up first. The market needs the registry's address at construction so it can resolve `winners` against authorized agents.

### 2. Register agents on chain

Each agent calls `AgentRegistry.register(passportHash, capabilities)`:

- `passportHash` = `keccak256(canonical_json(status_card))` (32 bytes).
- `capabilities` = pipe-delimited string. **Must contain `endpoint=<URL>`**, e.g. `"perps_liquidator|model=korai-8b|endpoint=https://demo.nunchi.trade/agents/agent-00/status.json"`.

The off-chain endpoint URL must serve the JSON body whose keccak matches `passportHash` — that's the binding that lets the chat layer trust an agent's transport pubkey.

### 3. Configure kora to talk to the chain

Pass `--chat-config <path>` to kora. The TOML/JSON file deserializes into [`ChatConfig`](src/service.rs); the relevant fields for on-chain wiring are inside the optional `chain` block ([`ChainConfig`](src/chain.rs)):

```toml
enabled       = true
me_seed       = 1                                  # ed25519 transport identity (POC; real keystore later)
bind_port     = 4101                               # chat mesh port (distinct from consensus)
bootstrappers = ["1@127.0.0.1:4101"]               # <seed>@<host:port>
registry_path = "/var/daeji/registry.json"         # file-backed authorized peer set

[chain]
rpc_ws         = "ws://127.0.0.1:8545"             # JSON-RPC WS endpoint
agent_registry = "0x..."                           # AgentRegistry deployment address
market         = "0x..."                           # MultiAgentMarket deployment address
my_controller  = "0x..."                           # this agent's EVM controller (for award detection)
from_block     = 0                                 # optional; defaults to chain head
```

What happens at runtime:

1. `kora --chat-config chat.toml ...` boots. The supervised spawn in `kora-service` calls `daeji_chat::service::run_chat(ctx, cfg)`.
2. `service::run_chat` pre-registers the lobby + 64 slot channels and spawns the chain watcher (because `chain` is set).
3. **`AgentRegistered` events** → watcher GETs the `endpoint=URL`, verifies `keccak256(body) == passportHash`, extracts `transport.pubkey`, writes a `Chain { ... }` record to `registry_path`. The 200ms registry poller picks it up and calls `oracle.track`, so commonware-p2p admits the new peer to the authenticated mesh.
4. **`JobAwarded` events** → watcher derives `room_id` locally, parity-checks against the on-chain `roomId`, logs the channel binding. The `JobAnnounce` on the lobby (sent by the coordinator after their tx confirms) is what actually activates the slot — auto-join from chain alone is deferred (Q-Open-6).

### 4. Kill switch

Set `--disable-chat` (or `DAEJI_CHAT_DISABLED=1` env var) on any kora instance to turn off chat at runtime without restarting consensus. Honored before any chat sockets open. Per canonical-plan §19 B2.3.

## Build + test

```sh
cargo test -p daeji-chat --lib    # 53 unit tests (room AEAD, cross-language parity, card schema, registry, lobby, supervisor)
cargo test --workspace            # full workspace — chat is one of many crates
cargo build -p daeji-chat         # builds lib only — no binaries
```

End-to-end live test (4 agents talking through this layer, settling on-chain via `submitMulti`) lives at `scripts/symphony-e2e.sh` in [`Nunchi-trade/contracts-core`](https://github.com/Nunchi-trade/contracts-core) PR #116.

## What lands in follow-up PRs

- **PR-Daeji-F**: room-key ECDH handshake (replaces the v1 plaintext `RoomKeyWrap` and the POC's `--room-key-hex` pre-share).
- **PR-Daeji-G**: per-job-type modules (ISFR, swarm, mining-bounty, etc.) per canonical-plan §11.
- **Q-Open-6**: lobby-driven auto-join from `JobAwarded` (currently the chain watcher logs awareness; the matching `JobAnnounce` activates the slot).

## Companion repos

- [`Nunchi-trade/contracts-core`](https://github.com/Nunchi-trade/contracts-core) — on-chain identity (`AgentRegistry.sol`), marketplace (`MultiAgentMarket.sol`), validator stack (`ConsortiumValidator.sol`, `IResolverTarget.sol`).
- [`Nunchi-trade/agent-chat`](https://github.com/Nunchi-trade/agent-chat) — canonical plan + spec docs (no Rust code).
