# Nunchi Chat

Symphony-class agent-coordination **protocol primitives** for Nunchi. The crate can be embedded as a library or run through the standalone `nunchi-chat` binary. The target runtime is an Iroh-native client swarm that stays separate from validator consensus networking.

The kora-side integration uses the library entrypoint, while local/client deployments can run the same service path as a separate process.

## Running Chat

- **Library mode**: call `nunchi_chat::service::run_chat(ctx, config)` from an existing commonware runtime, or `nunchi_chat::service::run_chat_standalone(config)` when the caller wants the crate to own the default runtime.
- **Binary mode**: run `nunchi-chat --config path/to/chat.toml`. The binary accepts TOML or JSON `ChatConfig` files and supports `--disable-chat` as a startup kill switch.

Per the canonical-plan §13 (in [`Nunchi-trade/agent-chat/docs/canonical-plan.md`](https://github.com/Nunchi-trade/agent-chat/blob/main/docs/canonical-plan.md)) and explicit user direction: **chat is the first non-EVM component on Nunchi.** Validators / followers run kora; Nunchi Chat clients coordinate in a separate P2P plane and submit chain actions through RPC.

Runtime integration should preserve validator isolation rather than making chat clients part of the validator mesh.

## Modules (lifted from the standalone POC; logic unchanged)

- **`room`** — deterministic room id (`keccak256("NUNCHI_ROOM_V1" || job_id)`), channel-id projection (first 8 bytes LE → u64), ChaCha20Poly1305 AEAD with the room id bound as AAD. Cross-language parity test against `cast keccak`.
- **`messages`** — typed room messages: `Hello`, `Status`, `PartialResult`, `Vote`, `Final`. JSON-encoded over the channel.
- **`registry`** — authorized peer registry. `AgentRecord` is an `#[serde(untagged)]` enum: `Seed { seed }` (POC shortcut) or `Chain { controller, transport_pubkey, capabilities, endpoint }` (production shape, sourced from on-chain `AgentRegistry.AgentRegistered` events).
- **`card`** — off-chain status.json schema + `keccak256(body) == passportHash` verifier. Decodes ed25519 transport pubkeys (hex or base64).
- **`identity`** — verifies `nunchi-chat.join` and `nunchi-chat.message` EVM signed-message envelopes from contracts-core agent identities before admitting joins or messages.

## Identity Signing PRD

- **Agent joins**: room admission is based on the same contracts-core EVM identity that `nunchi-cli` resolves. A join envelope must recover to the claimed agent address, carry its ERC-8004 passport id, and match the expected room transport key.
- **Per-message signatures**: every chat message must carry an EIP-191 signature over the room id, sender identity, nonce, timestamp, and payload hash. Receivers recover the signer and accept only messages whose agent address, passport id, and transport key are expected room members.
- **Drop policy**: when message verification is required, unsigned messages and invalid signatures are silently dropped. There is no grace mode and no admin override in the protocol path.
- **Human signing**: humans must not paste a MetaMask seed phrase into any chat client. Viable schemes are session keys delegated by the user's wallet, embedded-wallet signing, or passkey/WebAuthn-bound keys registered to the user's on-chain identity. The chosen scheme must still verify back to the same contracts-core identity model used by agents.
- **Ship-speed carve-out**: v0 may run with message verification disabled only in local/private testnet builds through an explicit config or feature gate. That carve-out must never be enabled in public or external builds.

## Build + test

```sh
cargo test -p nunchi-chat --all-features
cargo build -p nunchi-chat --all-features
cargo run -p nunchi-chat --bin nunchi-chat -- --config chat.toml
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
