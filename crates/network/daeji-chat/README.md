# daeji-chat

Symphony-class agent-coordination **protocol primitives** for Daeji. Library-only crate; **no standalone binaries**. Designed to be **bolted into the existing `kora` chain binary** (not run as a separate process), so chat lives on the same authenticated commonware mesh as consensus.

The kora-side integration (the `--enable-chat` flag, the service spawn-point, the chain-event background tasks) lands in a follow-up PR.

## Why a library + zero binaries

Per the canonical-plan §13 (in [`Nunchi-trade/agent-chat/docs/canonical-plan.md`](https://github.com/Nunchi-trade/agent-chat/blob/main/docs/canonical-plan.md)) and explicit user direction: **chat is the first non-EVM component on Daeji.** Validators / followers run kora; when chat is enabled, chat handlers run as additional commonware channels alongside consensus / mempool / blocks **on the same authenticated mesh**.

A separate `daeji-chat` binary process would mean a parallel mesh, separate peer-set sync, separate identity story. We don't want any of that. So this crate is just the protocol — the kora binary calls into it.

## Modules (lifted from the standalone POC; logic unchanged)

- **`room`** — deterministic room id (`keccak256("DAEJI_ROOM_V1" || job_id)`), channel-id projection (first 8 bytes LE → u64), ChaCha20Poly1305 AEAD with the room id bound as AAD. Cross-language parity test against `cast keccak`.
- **`messages`** — typed room messages: `Hello`, `Status`, `PartialResult`, `Vote`, `Final`. JSON-encoded over the channel.
- **`registry`** — authorized peer registry. `AgentRecord` is an `#[serde(untagged)]` enum: `Seed { seed }` (POC shortcut) or `Chain { controller, transport_pubkey, capabilities, endpoint }` (production shape, sourced from on-chain `AgentRegistry.AgentRegistered` events).
- **`card`** — off-chain status.json schema + `keccak256(body) == passportHash` verifier. Decodes ed25519 transport pubkeys (hex or base64).

## Build + test

```sh
cargo test -p daeji-chat --lib    # 17 unit tests (room AEAD + cross-language parity, card schema/verify, registry)
cargo build -p daeji-chat         # builds lib only — no binaries
```

## What lands in follow-up PRs

- **PR-Daeji-B**: integrate chat into `kora`. Adds:
  - `daeji-chat::service::run_chat(ctx, config)` — entry point the kora binary calls.
  - `--enable-chat` flag on `kora`.
  - Chain-event background tasks (subscribe to `AgentRegistry.AgentRegistered`, `MultiAgentMarket.JobAwarded`) running inside kora's tokio runtime.
  - Wires registry updates to kora's existing commonware peer-set oracle.
- **PR-Daeji-C**: room-key ECDH handshake (replaces the POC's `--room-key-hex` pre-share).
- **PR-Daeji-D+**: per-job-type modules (ISFR, swarm, mining-bounty, etc.) per canonical-plan §11.

## Companion repos

- [`Nunchi-trade/contracts-core`](https://github.com/Nunchi-trade/contracts-core) — on-chain identity (`AgentRegistry.sol`), marketplace (`MultiAgentMarket.sol`), validator stack (`ConsortiumValidator.sol`, `IResolverTarget.sol`).
- [`Nunchi-trade/agent-chat`](https://github.com/Nunchi-trade/agent-chat) — canonical plan + spec docs (no Rust code).
