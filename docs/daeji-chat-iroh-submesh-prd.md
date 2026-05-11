# PRD: Daeji Chat Iroh Sub-Mesh Shim

## Summary

Daeji chat should run as a dedicated peer-to-peer client swarm that is separate
from the validator consensus network. Chat clients gossip job coordination data
among themselves, then submit finalized chat-related transactions to the
validator network through RPC. Validators stay isolated from chat traffic, peer
churn, and client-side bandwidth spikes.

This PRD specifies a feature-flagged Iroh transport shim for the Daeji chat P2P
stack. The shim lets each active job room become its own Iroh gossip topic and
therefore its own sub-mesh. The existing Commonware transport remains the
default path while the Iroh transport is validated.

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

- Add a transport abstraction that can run the current Commonware chat transport
  or an Iroh-based transport.
- Preserve Commonware as the default transport for pre-testnet safety.
- Add an Iroh transport behind a cargo feature so each room maps to one Iroh
  gossip topic.
- Support NAT traversal through Iroh's QUIC and magicsock relay path for
  client-to-client discovery.
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
- Do not bridge Commonware and Iroh wire formats.
- Do not make Iroh the default transport until the spike is validated.

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

### Transport Abstraction

Introduce a `ChatTransport` interface with two implementations:

- `CommonwareTransport`: current chat behavior, preserved as the default.
- `IrohTransport`: new feature-flagged transport where each room is an Iroh
  gossip topic.

The service layer should depend on transport operations instead of direct
Commonware channel registration.

Required operations:

- Track the current authorized peer set for an oracle epoch.
- Send and receive lobby messages.
- Join a per-job room by room id, room key, and expected member list.
- Send and receive encrypted room messages through a room handle.
- Leave a room when the job concludes.

### Commonware Transport

The Commonware implementation should be a pure extraction of today's behavior:

- one well-known lobby channel
- fixed slot pool for active jobs
- hash-routed job slots
- AEAD decrypt attempt against active jobs in the slot
- registry-driven peer tracking

This implementation is the compatibility and rollback path.

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

- Peer allowlist: only current registry peers should be accepted or tracked.
- Room allowlist: room receivers drop messages from senders outside
  `expected_members`.

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

Add transport selection to chat configuration:

```toml
transport = "commonware" # default
iroh_relay = "https://relay.example.com" # optional
```

Expected cargo features:

```toml
default = ["transport-commonware"]
transport-commonware = []
transport-iroh = ["dep:iroh", "dep:iroh-gossip"]
```

The default build path should remain Commonware-only unless the Iroh feature is
explicitly enabled.

## Implementation Plan

### Milestone 1: Transport Interface

- Add `transport/mod.rs`.
- Define transport and room-handle traits or enum adapters.
- Add transport selection config.
- Keep default behavior unchanged.

Acceptance:

- Existing Commonware chat tests pass.
- Existing default build does not pull Iroh dependencies.

### Milestone 2: Commonware Extraction

- Move direct Commonware network setup out of the chat service.
- Preserve lobby behavior and fixed slot pool behavior.
- Keep registry polling and peer tracking behavior unchanged.

Acceptance:

- Diff is behavior-preserving.
- Demo driver still emits the same Hello, Status, and Final flow.
- No transport selection is required for existing operators.

### Milestone 3: Iroh Transport

- Add feature-flagged Iroh dependencies.
- Create endpoint and gossip instance.
- Subscribe to the well-known lobby topic at startup.
- Subscribe to one room topic per active job.
- Reuse existing room encryption and message serialization.

Acceptance:

- Two local Iroh chat clients can exchange one encrypted room message.
- One local Iroh lobby message can be sent and received.
- Commonware remains the default transport.

### Milestone 4: Isolation And Membership Tests

- Verify a Commonware peer and Iroh peer with the same room id do not see each
  other.
- Verify an uninvited Iroh node cannot produce accepted room messages.
- Verify a leaving room handle stops room traffic for a concluded job.

Acceptance:

- Cross-transport isolation is fail-closed.
- Room allowlist drops messages from unexpected senders.
- AEAD rejects wrong room keys and wrong room ids.

### Milestone 5: Operator Documentation

- Document how to enable `transport-iroh`.
- Document default relay behavior.
- Document validator isolation and RPC transaction submission.
- Document that the spike does not bridge transports.

Acceptance:

- Operators can run Commonware by default.
- Operators can opt into Iroh for devnet smoke tests.
- The validator network boundary is explicit.

## Verification Plan

- Unit tests for room encryption and transport message round trips.
- Local two-node Iroh smoke test with relay disabled when supported by the Iroh
  API.
- Local three-node Iroh smoke test for one room topic.
- Membership rejection test with a third uninvited node.
- Cross-transport isolation test.
- Latency baseline comparing local Commonware and local Iroh Hello-to-Final
  time.

## Rollout

1. Merge the PRD.
2. Implement the transport abstraction in a behavior-preserving PR.
3. Add Iroh behind `transport-iroh`.
4. Run local and devnet smoke tests.
5. Schedule a design review with chain engineering before enabling Iroh beyond
   controlled devnet runs.
6. Promote Iroh only after stability and operational relay strategy are proven.

## Risks

- Iroh API churn before a stable release may require updates in the shim.
- Iroh identity material may not map exactly to the existing chat transport key
  without a conversion layer.
- Relay defaults are acceptable for a spike but not for production operations.
- Runtime transport selection can be awkward in Rust if the trait uses
  associated room-handle types; an enum adapter may be required.
- NAT traversal behavior must be validated outside single-machine tests.

## Open Questions

- Which Iroh crate version should be pinned at implementation time?
- Should production relay URLs live in chat config, agent registry
  capabilities, or both?
- What RPC endpoint selection policy should chat clients use after the
  transport spike?
- Should lobby traffic eventually shard by network, market, or job class?
- What observability should expose per-room peer count, relay usage, and
  message latency?
