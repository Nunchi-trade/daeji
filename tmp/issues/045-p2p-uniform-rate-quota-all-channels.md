# 045: Uniform 1000 msg/s Rate Quota Across All P2P Channels Enables DoS

**Category:** security / performance
**Severity:** medium

**Labels:** `security`, `performance`, `p2p`

## Summary

All six P2P channels (votes, certs, resolver, blocks, backfill, tx_gossip) share the same rate quota of 1000 messages per second per peer. This one-size-fits-all limit is suboptimal because the channels carry fundamentally different traffic patterns with different trust levels and processing costs. In particular, the transaction gossip channel (which carries untrusted external user content) has the same rate limit as consensus-critical channels, enabling a misbehaving peer to consume bandwidth and CPU that could starve consensus message processing.

## Problem

A single `default_quota()` function defines the rate limit for all channels, and the `build_with_quota()` method applies this same quota to every channel registration.

The default quota at `crates/network/transport/src/builder.rs:22-24`:

```rust
/// Default rate quota for channels (1000 messages per second).
const fn default_quota() -> Quota {
    Quota::per_second(NonZeroU32::new(1000).expect("1000 is non-zero"))
}
```

All six channels registered with the same quota at `crates/network/transport/src/builder.rs:83-92`:

```rust
// Register simplex channels (consensus: high frequency, small messages)
let votes = network.register(CHANNEL_VOTES, quota, consensus_backlog);
let certs = network.register(CHANNEL_CERTS, quota, consensus_backlog);
let resolver = network.register(CHANNEL_RESOLVER, quota, resolver_backlog);

// Register marshal channels (blocks: large messages, backfill: burst-heavy)
let blocks = network.register(CHANNEL_BLOCKS, quota, block_backlog);
let backfill = network.register(CHANNEL_BACKFILL, quota, resolver_backlog);

// Register transaction gossip channel
let tx_gossip_channel = network.register(CHANNEL_TX_GOSSIP, quota, gossip_backlog);
```

The problems with this uniform quota:

1. **Consensus channels (votes, certs)**: At 30+ blocks/s with 10 validators, vote traffic alone can be 300+ msgs/s per peer. The 1000/s limit provides only ~3x headroom, which may be insufficient during view changes or network recovery when burst rates spike.

2. **Block channel**: Carries large messages (potentially megabyte-sized blocks). 1000 blocks/s is far more than needed. A lower message-count quota would be appropriate, but a byte-rate limiter would be more effective since the concern is bandwidth, not message count.

3. **Transaction gossip**: This is the most abuse-prone channel since transaction content originates from untrusted external users. A malicious peer could send 1000 transactions/s at up to the max message size (1 MB per `DEFAULT_MAX_MESSAGE_SIZE`), consuming up to 1 GB/s of bandwidth per peer connection.

## Impact

A misbehaving or compromised peer can maximize its impact by flooding the transaction gossip channel (least trusted, most expensive to process due to signature recovery and state lookups). This consumes:

- **Bandwidth**: Up to 1000 messages/s * message size per peer
- **CPU**: Each gossipped transaction requires RLP decoding, signature recovery (ECDSA), and state lookups (nonce, balance) -- see `crates/node/runner/src/runner.rs:1112-1123`
- **Lock contention**: Each gossipped transaction acquires the `LedgerView` mutex for state lookups (see issue 048)

This load can starve consensus-critical message processing (votes, certs) on the same node, potentially causing voting delays and nullification. The consensus channels need high rate limits for legitimate traffic, but the gossip channel should have a much lower limit since it carries untrusted content.

## Root Cause

The rate quota was set to a single uniform value for simplicity during development. The `build()` method delegates to `build_with_quota()` with `default_quota()`, and while `build_with_quota()` accepts a custom quota, it still applies the same quota to all channels rather than accepting per-channel quotas.

## Suggested Fix

**Option 1 (recommended):** Use per-channel quotas calibrated to expected traffic patterns. Modify `build_with_quota` to accept per-channel quotas or add a new builder method:

```rust
pub struct ChannelQuotas {
    /// Consensus channels: high burst tolerance, trusted peers only
    pub consensus: Quota,     // e.g., 3000 msg/s
    /// Block broadcast: low frequency but large messages
    pub blocks: Quota,        // e.g., 100 msg/s
    /// Resolver/backfill: burst during catch-up but bounded
    pub sync: Quota,          // e.g., 500 msg/s
    /// Transaction gossip: most abuse-prone, limit aggressively
    pub gossip: Quota,        // e.g., 200 msg/s
}

// Then in build_with_quotas():
let votes = network.register(CHANNEL_VOTES, quotas.consensus, consensus_backlog);
let certs = network.register(CHANNEL_CERTS, quotas.consensus, consensus_backlog);
let resolver = network.register(CHANNEL_RESOLVER, quotas.sync, resolver_backlog);
let blocks = network.register(CHANNEL_BLOCKS, quotas.blocks, block_backlog);
let backfill = network.register(CHANNEL_BACKFILL, quotas.sync, resolver_backlog);
let tx_gossip = network.register(CHANNEL_TX_GOSSIP, quotas.gossip, gossip_backlog);
```

**Option 2:** Add a byte-rate limiter for the transaction gossip channel in addition to the message-rate limiter, to prevent bandwidth exhaustion from large transactions.

## Files to Modify

- `crates/network/transport/src/builder.rs` -- `default_quota()` at line 22 and `build_with_quota()` at line 70
- `crates/network/transport/src/config.rs` -- Add per-channel quota configuration fields to `TransportConfig`

## Related Issues

- `047-p2p-max-message-size-inconsistent-with-block-codec.md` -- MAX_MESSAGE_SIZE inconsistency affects bandwidth impact of rate quotas
- `048-p2p-per-tx-state-fetch-gossip-handler.md` -- Per-transaction state fetch amplifies the CPU cost of gossip floods
