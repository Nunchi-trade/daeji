# daeji-chat

Symphony-class agent-coordination message bus for Daeji. Built on `commonware-p2p::authenticated::discovery` with on-chain identity binding via [`Nunchi-trade/contracts-core`](https://github.com/Nunchi-trade/contracts-core).

This crate is the **canonical home** for the chat protocol library + binaries. It can be:

1. **Embedded inside `kora`** (the daeji node binary) — validators / followers run chat alongside consensus on the same authenticated commonware mesh. Off by default; opt-in via feature flag (`kora --features chat`, future PR).
2. **Run standalone** — light-client agents that don't run kora invoke the binaries directly (`cargo run --bin daeji-chat -- --me=...`). The standalone binary in `Nunchi-trade/agent-chat` is a thin wrapper that depends on this crate.

Both modes speak the same protocol on the same authenticated mesh.

## Architecture

The kinds-of-gossip matrix (full version in [`Nunchi-trade/agent-chat/docs/canonical-plan.md`](https://github.com/Nunchi-trade/agent-chat/blob/main/docs/canonical-plan.md)):

| Job type | Wire mode | Settlement contract |
|---|---|---|
| Symphony | Room-keyed (X25519 ECDH → ChaCha20Poly1305) | `MultiAgentMarket.sol` |
| ISFR | Public broadcast | `ISFROracle.sol` (planned) |
| Reputation-gated | Room-keyed | `MultiAgentMarket.sol` + tier filter |
| Swarm / mirror-fish | Commit-reveal AEAD | `SwarmEvaluator.sol` (planned) |
| MEV / funding-rate race | Public reveal after deadline | direct chain submission |
| Self-learning / autoresearch | Public broadcast (CRDT) | `KnowledgeBoard.sol` (planned) |
| Mining-bounty | Room-keyed | extended `MultiAgentMarket.sol` |

## Modules

- `room` — deterministic room id (`keccak256("DAEJI_ROOM_V1" || job_id)`), channel id projection, ChaCha20Poly1305 AEAD with the room id bound as AAD. Cross-language parity test against `cast keccak`.
- `messages` — typed room messages: `Hello`, `Status`, `PartialResult`, `Vote`, `Final`. JSON-encoded over the channel.
- `registry` — authorized peer registry. `AgentRecord` is an `#[serde(untagged)]` enum: `Seed { seed }` (POC shortcut) or `Chain { controller, transport_pubkey, capabilities, endpoint }` (production shape, sourced from `AgentRegistry.AgentRegistered` events).
- `card` — off-chain status.json schema + `keccak256(body) == passportHash` verifier. Decodes ed25519 transport pubkeys (hex or base64).

## Binaries

- `daeji-chat` — agent runtime. Reads the registry on startup, polls every 200ms, calls `oracle.track(epoch, set)` on epoch change.
- `daeji-indexer` — registry editor CLI (`init` / `seed` / `add` / `remove` / `show`). Stand-in until the chain-event-driven indexer lands.
- `daeji-watch-jobs` — alloy-based subscriber. Watches `MultiAgentMarket.JobAwarded` events on a configurable RPC and prints the channel-id binding.

## Build + test

```sh
cargo test -p daeji-chat --lib    # 17 unit tests (room AEAD + cross-language parity, card schema/verify, registry)
cargo build -p daeji-chat         # builds lib + 3 binaries
```

## 3-node dynamic-membership smoke test

```sh
ROOM_KEY=$(printf '%064d' 7)
REG=/tmp/daeji-registry.json

# 1) Seed the registry with two agents.
./target/release/daeji-indexer --path $REG seed 1,2

# 2) Start A (bootstrapper) and B.
./target/release/daeji-chat --me=1@4101 --registry-path=$REG \
    --job-id=demo --room-key-hex=$ROOM_KEY &
./target/release/daeji-chat --me=2@4102 --registry-path=$REG \
    --bootstrappers=1@127.0.0.1:4101 \
    --job-id=demo --room-key-hex=$ROOM_KEY &

# 3) Register a third agent. Within ~200ms A and B refresh oracle.track.
./target/release/daeji-indexer --path $REG add 3

# 4) Start C with --drive. Mesh now has 3 agents.
./target/release/daeji-chat --me=3@4103 --registry-path=$REG \
    --bootstrappers=1@127.0.0.1:4101 \
    --job-id=demo --room-key-hex=$ROOM_KEY \
    --drive --drive-after-secs=2 &
```

## Companion repos

- [`Nunchi-trade/contracts-core`](https://github.com/Nunchi-trade/contracts-core) — on-chain identity (`AgentRegistry.sol`), marketplace (`MultiAgentMarket.sol`), validator stack (`ConsortiumValidator.sol`, `IResolverTarget.sol`).
- [`Nunchi-trade/agent-chat`](https://github.com/Nunchi-trade/agent-chat) — standalone-binary entry point + canonical plan / docs.
