# PRD: Daeji Chat Iroh Sub-Mesh Shim

## Summary

Daeji chat should run as a dedicated peer-to-peer client swarm that is separate
from the validator consensus network. Chat clients gossip job coordination data
among themselves, then submit finalized chat-related transactions to the
validator network through RPC. Validators stay isolated from chat traffic, peer
churn, and client-side bandwidth spikes.

This PRD specifies an Iroh-native transport for the Daeji chat P2P stack. Each
active job room becomes its own Iroh gossip topic and therefore its own
sub-mesh. Chat does not need to preserve or reimplement the existing Commonware
P2P path; Commonware remains relevant to validator networking, not chat-room
coordination.

## Problem

The current chat direction risks coupling chat-client traffic to the validator
mesh. That is undesirable for three reasons:

- Validator nodes should not have to accept arbitrary chat-client peer churn or
  all-to-all traffic.
- Job rooms should not all share a small fixed pool of transport channels where
  unrelated jobs can collide and compete for bandwidth.
- Production chat clients should not require every participant to advertise a
  public IP address.

The target architecture is:

1. Chat clients form their own authenticated P2P swarm.
2. Each job room becomes a dedicated sub-mesh inside that swarm.
3. Chat clients submit resulting chain actions to validators through RPC.
4. The validator network remains isolated and only sees normal RPC-submitted
   transactions.

## Goals

- Make the chat P2P stack Iroh-native instead of reimplementing Commonware P2P.
- Map each room to one Iroh gossip topic.
- Support NAT traversal through Iroh's QUIC and magicsock relay path for
  client-to-client discovery.
- Bind chat participant identity to the existing `contracts-core` ERC-8004
  agent identity model instead of treating transport public keys as standalone
  identities.
- Keep room confidentiality and access proof at the application layer using the
  existing room key AEAD.
- Keep validator consensus networking out of the chat-client swarm.
- Document the operational model where chat clients submit chain transactions
  through RPC instead of joining the validator mesh.

## Non-Goals

- Do not replace validator consensus networking.
- Do not make validators relay chat messages.
- Do not add the chain-side `ChatTx` or channel primitive in this PR.
- Do not add a multi-validator RPC fanout abstraction in this PR.
- Do not implement room-key ECDH wrapping in this PR.
- Do not bridge Commonware and Iroh wire formats for chat.
- Do not preserve a Commonware chat-transport fallback in this workstream.

## Users

- Agent operators running Daeji chat clients.
- Validators that should remain insulated from chat-client traffic.
- Requesters and agents coordinating per-job execution rooms.
- Chain engineers reviewing the boundary between chat networking and validator
  networking.

## Architecture

### Network Boundary

The system has two separate network planes:

- Chat plane: client-operated P2P swarm used for lobby messages, job room
  gossip, and agent coordination.
- Validator plane: validator-operated consensus network used for block
  production, finality, and execution.

Chat clients do not join the validator P2P mesh. When a chat flow needs to
affect chain state, the chat client submits a transaction through an RPC
endpoint. The RPC node is the boundary between chat coordination and validator
execution.

### Iroh-Native Chat Transport

Introduce a chat transport boundary backed by Iroh. The service layer should
depend on transport operations instead of direct Commonware channel
registration. The chat path does not need a Commonware implementation.

Required operations:

- Track the current authorized peer set for an oracle epoch.
- Send and receive lobby messages.
- Join a per-job room by room id, room key, and expected member list.
- Send and receive encrypted room messages through a room handle.
- Leave a room when the job concludes.

### Agent Identity Binding

Chat participant identity should be chain-derived from `contracts-core`, not
invented by the chat transport. The canonical participant id is:

- agent EVM address from `AgentRegistry`
- `passportId` from ERC-8004 `IdentityRegistry.ownerToPassportId(agent)`
- `passportHash` from `AgentRegistry.getAgent(agent)`
- liveness from `AgentRegistry.isActive(agent)`

The transport public key is an authenticated routing key under that agent
identity. It is not the top-level identity.

The chat registry should therefore be derived from `contracts-core` state:

1. Watch `AgentRegistry.AgentRegistered(agent, passportHash, capabilities)`.
2. Resolve the agent card or capability payload referenced by `capabilities`.
3. Extract the chat transport public key, supported transports, relay hints, and
   RPC endpoint hints from that metadata.
4. Verify the metadata against `passportHash`.
5. Confirm the agent owns a passport through `IdentityRegistry`.
6. Include the `(agent address, passportId, transport pubkey)` tuple in the chat
   allowlist only while `AgentRegistry.isActive(agent)` is true.

Room membership should be expressed in terms of agent addresses or passport ids.
The transport layer receives the corresponding transport pubkeys only after the
identity resolver has checked the on-chain identity and liveness constraints.

This creates a clear identity split:

- contracts-core identity: who the participant is
- chat registry: which transport key that identity currently uses
- Iroh transport: how bytes are routed

### Iroh Transport

The Iroh implementation owns one endpoint and one gossip instance:

- Lobby topic: `keccak256("DAEJI_LOBBY_V1")`
- Room topic: the 32-byte `room_id`
- Endpoint identity: derived from the same transport identity material used by
  the chat client
- Relay mode: default public relay for the spike, configurable relay URL for
  operator-managed relay infrastructure later

Each active job room subscribes to a distinct topic. This gives the desired
sub-mesh property: peers for one job do not share a transport room with peers
for unrelated jobs.

### Membership And Confidentiality

Iroh gossip topics are discoverable by topic id, so transport membership is not
the sole security boundary. Access control is enforced in two layers:

- Peer allowlist: only current registry peers derived from active
  `contracts-core` agent identities should be accepted or tracked.
- Room allowlist: room receivers drop messages from senders outside
  `expected_members`, where expected members are resolved from agent
  address/passport identity into transport pubkeys.

Message confidentiality and access proof remain application-layer concerns.
Room messages continue to be serialized, encrypted with ChaCha20-Poly1305, and
bound to the room id as AEAD additional authenticated data. A peer that does not
have the room key cannot produce a valid room message.

### Transaction Submission

The chat swarm does not finalize chain state by itself. For chain-visible
actions, clients submit transactions to the validator network through RPC.
Examples include future chat transaction types, room lifecycle transactions, or
job conclusion transactions.

This preserves the isolation boundary:

- Chat clients gossip off-chain coordination data.
- RPC accepts signed transactions.
- Validators process transactions through normal block production.

## Configuration

Add Iroh chat transport configuration:

```toml
iroh_relay = "https://relay.example.com" # optional
agent_registry = "0x..." # contracts-core AgentRegistry
identity_registry = "0x..." # contracts-core IdentityRegistry
```

Expected cargo features:

```toml
default = ["transport-iroh"]
transport-iroh = ["dep:iroh", "dep:iroh-gossip"]
```

The chat crate should compile with Iroh chat transport as the default direction.

## Implementation Plan

### Milestone 1: Iroh Chat Transport Boundary

- Add `transport/mod.rs`.
- Define transport and room-handle traits backed by Iroh.
- Add Iroh relay and identity resolver config.
- Remove the requirement to mirror Commonware's fixed slot-pool model for chat.

Acceptance:

- Existing chat message, room, lobby, and registry tests pass.
- Rooms map to one Iroh topic per 32-byte room id.

### Milestone 1.5: Contracts-Core Identity Resolver

- Add a chain-backed identity resolver for `contracts-core` agent identities.
- Resolve chat participants from `AgentRegistry` plus `IdentityRegistry`.
- Treat transport pubkeys as routing keys bound to an agent address and
  passport id.
- Filter inactive agents using `AgentRegistry.isActive`.

Acceptance:

- A registered active agent with a valid passport resolves to one chat
  participant record.
- An inactive, unregistered, or passportless agent is excluded from the chat
  allowlist.
- Room expected members are expressed as agent identities before being lowered
  to transport pubkeys.

### Milestone 2: Iroh Lobby And Room Topics

- Replace direct Commonware channel setup in the chat service.
- Subscribe to the well-known Iroh lobby topic at startup.
- Subscribe to one Iroh room topic per active job.
- Keep registry polling and peer tracking behavior as the source of the
  transport allowlist.

Acceptance:

- Chat no longer depends on the fixed 64-slot chat channel pool.
- Demo driver still emits the same Hello, Status, and Final flow.
- No chat transport selection is required for existing operators; chat uses Iroh.

### Milestone 3: Iroh Transport

- Add Iroh dependencies for the chat transport path.
- Create endpoint and gossip instance.
- Subscribe to the well-known lobby topic at startup.
- Subscribe to one room topic per active job.
- Reuse existing room encryption and message serialization.

Acceptance:

- Two local Iroh chat clients can exchange one encrypted room message.
- One local Iroh lobby message can be sent and received.
- Iroh is the chat transport path.

### Milestone 4: Isolation And Membership Tests

- Verify an uninvited Iroh node cannot produce accepted room messages.
- Verify a leaving room handle stops room traffic for a concluded job.

Acceptance:

- Room allowlist drops messages from unexpected senders.
- AEAD rejects wrong room keys and wrong room ids.

### Milestone 5: Operator Documentation

- Document how to run the Iroh chat transport.
- Document default relay behavior.
- Document validator isolation and RPC transaction submission.
- Document that chat no longer preserves the fixed Commonware slot-pool model.

Acceptance:

- Operators can run Iroh chat devnet smoke tests.
- The validator network boundary is explicit.

## Verification Plan

- Unit tests for room encryption and transport message round trips.
- Local two-node Iroh smoke test with relay disabled when supported by the Iroh
  API.
- Local three-node Iroh smoke test for one room topic.
- Membership rejection test with a third uninvited node.
- Identity resolver test where a `contracts-core` registered active agent is
  accepted and an inactive or passportless agent is rejected.
- Latency baseline for local Iroh Hello-to-Final time.

## Rollout

1. Merge the PRD.
2. Implement the Iroh chat transport boundary.
3. Wire the chat service to the Iroh lobby and per-room topics.
4. Run local and devnet smoke tests.
5. Schedule a design review with chain engineering before enabling Iroh beyond
   controlled devnet runs.
6. Promote Iroh only after stability and operational relay strategy are proven.

## Risks

- Iroh API churn before a stable release may require updates in the shim.
- Iroh identity material may not map exactly to the existing chat transport key
  without a conversion layer.
- Agent metadata can drift from on-chain identity state if clients cache
  resolved transport keys too long.
- Relay defaults are acceptable for a spike but not for production operations.
- NAT traversal behavior must be validated outside single-machine tests.

## Open Questions

- Which Iroh crate version should be pinned at implementation time?
- Should production relay URLs live in chat config, agent registry
  capabilities, or both?
- Should the chat transport pubkey live in `AgentRegistry.capabilities`, the
  hashed agent card referenced by `passportHash`, or both?
- Should room membership use agent EVM address or ERC-8004 `passportId` as the
  canonical room member id?
- What RPC endpoint selection policy should chat clients use after the
  transport spike?
- Should lobby traffic eventually shard by network, market, or job class?
- What observability should expose per-room peer count, relay usage, and
  message latency?
