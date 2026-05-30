# PRD: Iroh-Backed Validator Network Adapter

## Summary

This PRD explores replacing the current validator P2P substrate with an
Iroh-backed transport while preserving the consensus-visible behavior that Kora
expects today. The goal is not to move validator traffic onto gossip. The goal
is to use Iroh for authenticated QUIC connections, peer discovery, relay
rendezvous, and NAT traversal, while keeping validator communication as explicit
all-to-all fanout over the current validator set.

This is intentionally separate from the Daeji chat Iroh sub-mesh PRD. Chat can
use Iroh gossip topics because per-job rooms benefit from sub-mesh semantics.
Consensus traffic should first preserve deterministic validator-set delivery and
peer accounting.

## Problem

The current P2P layer is built around Commonware's authenticated discovery
network. That gives Kora the channel-oriented API it needs, but production
operators still have open questions around public IP exposure, NAT traversal,
and how ergonomic it will be to operate validator networking across real
machines.

Iroh may improve the operational networking layer:

- Validators can identify each other by stable node identity instead of
  operator-managed public address assumptions.
- QUIC and magicsock can handle direct paths, relay rendezvous, and NAT
  traversal.
- Relay fallback can reduce pressure to expose every validator's public IP
  directly.
- The same underlying substrate could later support isolated chat sub-meshes
  without coupling chat clients to the validator mesh.

The hard requirement is that consensus-critical traffic must not accidentally
become best-effort gossip. Consensus should keep explicit delivery semantics to
the current validator set.

## Goals

- Define what an Iroh-backed validator transport would look like before any code
  changes.
- Preserve the current channel/peer-set semantics exposed to consensus and
  marshal layers.
- Preserve explicit all-to-all validator fanout for consensus-critical traffic.
- Use Iroh for connection establishment, encrypted QUIC streams, NAT traversal,
  and relay fallback.
- Keep chat Iroh sub-mesh work separate from validator networking work.
- Identify the adapter surface needed to test Iroh without rewriting consensus.

## Non-Goals

- Do not switch consensus messages to Iroh gossip.
- Do not rewrite Simplex, marshal, or execution code.
- Do not make chat clients members of the validator network.
- Do not remove Commonware transport until an Iroh adapter proves equivalent.
- Do not decide the final production relay topology in this PRD.
- Do not implement code in this PR.

## Users

- Validator operators running Daeji/Kora nodes across real networks.
- Chain engineers maintaining consensus and networking safety.
- Infrastructure operators responsible for relay or bootstrap nodes.
- Chat and agent-network engineers who need a future substrate that can support
  isolated non-validator meshes.

## Architecture

### Network Planes

Daeji should keep separate network planes:

- Validator plane: one authenticated Iroh-backed validator network keyed by the
  validator set.
- Chat plane: one chat client swarm plus one sub-mesh per active chat room.

This PRD covers only the validator plane. The chat plane remains covered by the
separate chat Iroh sub-mesh PRD.

### Adapter Shape

The first implementation should be an adapter, not a native rewrite. The adapter
should preserve the current Kora transport shape:

- register named or numeric channels
- track the current authorized peer set by epoch
- send bytes to one peer, a subset of peers, or all tracked peers
- receive bytes with authenticated sender identity
- expose backpressure, message-size limits, and metrics
- fail closed when a sender is not in the current peer set

Under the hood, the adapter would use Iroh endpoint connections and QUIC streams
instead of Commonware discovery sockets.

### Validator Identity

Validator identity should be derived from the validator set, not from opportunistic
Iroh discovery. A validator is eligible only if it is in the current consensus
peer set for the epoch.

The adapter should maintain a mapping:

```text
validator identity -> iroh NodeId -> active connection state
```

Open question: whether the Iroh `NodeId` can be directly derived from the
existing validator ed25519 key material, or whether Kora needs an explicit
validator-id-to-node-id binding in config or chain state.

### All-To-All Preservation

For consensus-critical channels, broadcast should mean deterministic fanout to
every validator in the current peer set:

```text
for peer in current_validator_set:
    send(peer, channel_id, message)
```

Iroh can choose the best path for each peer connection:

- direct QUIC when hole punching succeeds
- relay-assisted rendezvous when direct path is not yet available
- relay fallback when direct connectivity is impossible

The consensus layer should not need to know which path was used. It should still
observe per-peer sends, receives, errors, and metrics.

### Channel Multiplexing

The adapter should map existing transport channels onto Iroh streams or framed
datagrams. A conservative shape:

- one long-lived Iroh connection per validator peer
- framed messages carrying `channel_id`, sequence metadata, and payload length
- per-channel inbound queues matching current channel registration behavior
- bounded queues and explicit backpressure

This avoids relying on one Iroh gossip topic per consensus channel and keeps
delivery accounting per validator peer.

### Peer-Set Updates

On epoch change:

1. Add newly authorized validators to the target peer set.
2. Dial or accept Iroh connections for newly authorized peers.
3. Stop accepting messages from removed validators immediately.
4. Drain or close removed validator connections after a short grace period.
5. Update metrics so operators can see missing, relay-only, and direct peers.

Fail-closed behavior matters more than smoothness. A removed validator must not
continue delivering consensus messages after the local peer set advances.

### Relay Strategy

For the spike, relay mode can use Iroh defaults or a devnet relay. Production
should likely run Daeji-controlled relay infrastructure.

The relay should be treated as connectivity infrastructure, not a trust anchor.
Validator messages remain encrypted and authenticated end-to-end between
validator identities.

## Relationship To Chat

The chat sub-mesh PRD and this validator PRD are complementary:

- Validator network: one authenticated Iroh-backed network keyed by validator
  set, preserving explicit all-to-all fanout.
- Chat network: one chat swarm with Iroh gossip topics for lobby and per-room
  sub-meshes.

This separation keeps chat peer churn and room traffic out of the validator
network. It also leaves room to revisit transaction submission. The chat PRD's
RPC submission path is a conservative first boundary; a later design could
submit transactions through an Iroh-backed validator ingress path once this
adapter exists.

## Implementation Plan

### Milestone 1: Transport Equivalence Spec

- Inventory the current `kora-transport` API and channel semantics.
- Define required delivery, backpressure, error, and metrics behavior.
- Identify which semantics are consensus-critical versus implementation detail.

Acceptance:

- Engineers can compare Commonware and Iroh behavior against the same checklist.
- Consensus-facing API changes are either zero or explicitly listed.

### Milestone 2: Iroh Identity Binding

- Decide how validator identity maps to Iroh `NodeId`.
- Define config or chain-state fields needed to bind identities.
- Define peer-set admission and removal rules.

Acceptance:

- Unknown Iroh nodes cannot deliver validator-channel messages.
- Removed validators fail closed after epoch advancement.
- Operator config makes identity binding auditable.

### Milestone 3: Prototype Adapter

- Build an optional Iroh-backed implementation of the transport API.
- Preserve channel registration and sender/receiver shape.
- Implement explicit all-to-all fanout for validator broadcasts.
- Keep Commonware as the default transport.

Acceptance:

- Existing consensus code compiles against the adapter with minimal or no
  changes.
- Local multi-validator devnet can start with Iroh transport enabled.
- Default build path remains unchanged.

### Milestone 4: Failure And Network Testing

- Test direct local connections.
- Test validators behind NAT using relay rendezvous.
- Drop one validator mid-run and verify remaining validators continue.
- Remove one validator from the peer set and verify fail-closed behavior.
- Compare latency and throughput to the Commonware transport.

Acceptance:

- Direct and relay-assisted paths both work.
- Consensus traffic remains per-peer accountable.
- Iroh transport is no worse than a documented latency threshold for local
  devnet.

### Milestone 5: Production Readiness Review

- Decide relay topology.
- Define observability dashboards.
- Decide whether Iroh can replace Commonware transport or should remain an
  optional backend.
- Review findings with Commonware and chain engineering.

Acceptance:

- A go/no-go decision can be made from measured behavior.
- Remaining gaps are tracked before any default transport change.

## Verification Plan

- Unit tests for channel framing and sender authentication.
- Integration tests for all-to-all broadcast over a local validator set.
- Epoch-change tests for added and removed validators.
- NAT and relay smoke tests across containers or machines.
- Metrics comparison between Commonware and Iroh transports.
- Long-running devnet test with validator restarts.

## Risks

- Iroh may not expose exactly the connection lifecycle hooks needed to mirror
  Commonware's authenticated peer oracle.
- Validator identity may require an explicit binding layer if existing ed25519
  keys cannot directly produce Iroh node identities.
- Relay fallback can hide connectivity problems unless metrics distinguish
  direct, relay-assisted, and relay-only peers.
- Consensus safety depends on preserving per-peer delivery semantics; using
  gossip too early would blur those guarantees.
- API churn in Iroh could make the adapter expensive to maintain until versions
  stabilize.

## Open Questions

- Can Iroh `NodeId` be derived from current validator key material, or do we need
  a separate Iroh keypair per validator?
- Should validator Iroh endpoints be configured off-chain, derived from chain
  state, or distributed through a bootstrap registry?
- Which current channels require strict all-to-all delivery versus best-effort
  propagation?
- What relay topology is acceptable for devnet, testnet, and production?
- Should transaction ingress eventually use the validator Iroh network directly
  instead of RPC submission?
- What does the Commonware team recommend as the cleanest adapter boundary?
