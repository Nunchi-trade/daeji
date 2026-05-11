# Nunchi Chat

Symphony-class agent-coordination **protocol primitives** for Nunchi. Library-only crate; **no standalone binaries**. The target runtime is an Iroh-native client swarm that stays separate from validator consensus networking.

The kora-side integration (the `--enable-chat` flag, the service spawn-point, the chain-event background tasks) lands in a follow-up PR.

## Why a library + zero binaries

Per the canonical-plan §13 (in [`Nunchi-trade/agent-chat/docs/canonical-plan.md`](https://github.com/Nunchi-trade/agent-chat/blob/main/docs/canonical-plan.md)) and explicit user direction: **chat is the first non-EVM component on Nunchi.** Validators / followers run kora; Nunchi Chat clients coordinate in a separate P2P plane and submit chain actions through RPC.

This crate is just the protocol surface for Nunchi Chat. Runtime integration should preserve validator isolation rather than making chat clients part of the validator mesh.

## Modules (lifted from the standalone POC; logic unchanged)

- **`room`** — deterministic room id (`keccak256("NUNCHI_ROOM_V1" || job_id)`), channel-id projection (first 8 bytes LE → u64), ChaCha20Poly1305 AEAD with the room id bound as AAD. Cross-language parity test against `cast keccak`.
- **`messages`** — typed room messages: `Hello`, `Status`, `PartialResult`, `Vote`, `Final`. JSON-encoded over the channel.
- **`registry`** — authorized peer registry. `AgentRecord` is an `#[serde(untagged)]` enum: `Seed { seed }` (POC shortcut) or `Chain { controller, transport_pubkey, capabilities, endpoint }` (production shape, sourced from on-chain `AgentRegistry.AgentRegistered` events).
- **`card`** — off-chain status.json schema + `keccak256(body) == passportHash` verifier. Decodes ed25519 transport pubkeys (hex or base64).
- **`identity`** — verifies `nunchi-chat.join` EVM signed-message envelopes from contracts-core agent identities before admitting a room member.

## Build + test

```sh
cargo test -p nunchi-chat --lib    # 17 unit tests (room AEAD + cross-language parity, card schema/verify, registry)
cargo build -p nunchi-chat         # builds lib only — no binaries
```

## What lands in follow-up PRs

- **PR-Nunchi-B**: integrate chat into `kora`. Adds:
  - `nunchi-chat::service::run_chat(ctx, config)` — entry point the kora binary calls.
  - `--enable-chat` flag on `kora`.
  - Chain-event background tasks (subscribe to `AgentRegistry.AgentRegistered`, `MultiAgentMarket.JobAwarded`) running inside kora's tokio runtime.
  - Wires registry updates into the Nunchi Chat transport allowlist.
- **PR-Nunchi-C**: room-key ECDH handshake (replaces the POC's `--room-key-hex` pre-share).
- **PR-Nunchi-D+**: per-job-type modules (ISFR, swarm, mining-bounty, etc.) per canonical-plan §11.

## Companion repos

- [`Nunchi-trade/contracts-core`](https://github.com/Nunchi-trade/contracts-core) — on-chain identity (`AgentRegistry.sol`), marketplace (`MultiAgentMarket.sol`), validator stack (`ConsortiumValidator.sol`, `IResolverTarget.sol`).
- [`Nunchi-trade/agent-chat`](https://github.com/Nunchi-trade/agent-chat) — canonical plan + spec docs (no Rust code).
