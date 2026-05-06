# Tempo Transaction Submission Research

This note captures validated findings about Tempo's transaction submission and
transaction pool behavior, with an emphasis on details that may matter when
comparing Kora's RPC and txpool design against Tempo.

## Summary

Tempo accepts transactions through the standard Ethereum JSON-RPC
`eth_sendRawTransaction` method. Standard EVM transactions use the normal
Reth-style transaction pool path, while Tempo's native transaction type has
custom handling for 2D nonce account-abstraction transactions.

Tempo also has a distinct subblock/proposer-targeted submission path. Official
RPC documentation says transactions targeting a subblock proposer are routed
directly to the consensus layer when submitted to the matching validator node,
and rejected by other nodes. The public source supports this mechanism: subblock
transactions are identified through a reserved `nonce_key` prefix, and the
remaining nonce-key bytes encode a partial validator key.

## Validated Details

- Public Tempo RPC is live at `https://rpc.tempo.xyz`.
- `eth_chainId` on the public RPC returns `0x1079` (`4217`).
- `tempo_forkSchedule` is available on the public RPC and currently reports
  `T3` as active.
- `txpool_status` is not allowed on the public RPC, which matches Tempo's
  documentation caveat that not all exposed method groups are available on
  public endpoints.
- Tempo's native transaction type is `0x76` in both the transaction spec and
  current source (`TEMPO_TX_TYPE_ID`).
- The official RPC page currently refers to Tempo Transactions as type `0x54`;
  this appears stale or incorrect relative to the transaction spec and source.
- Subblock transactions use `TEMPO_SUBBLOCK_NONCE_KEY_PREFIX = 0x5b`.
- The source derives `subblock_proposer()` from bytes in the transaction
  `nonce_key`, producing a partial validator key.

## Mempool and Txpool Behavior

Tempo's custom account-abstraction pool (`AA2dPool`) tracks 2D nonce
transactions separately from the standard protocol pool.

- 2D nonce transactions are pending only when the relevant nonce sequence has no
  gap from the current on-chain nonce.
- Future nonce transactions are queued until earlier nonces arrive or are mined.
- Expiring-nonce transactions are treated as always pending/independent.
- The pool tracks per-sender transaction counts for DoS protection.
- Eviction removes queued transactions before pending transactions.
- Eviction is priority-based using Reth `CoinbaseTipOrdering`.
- Because Tempo has a constant base fee, priority is fixed at insertion.
- When priorities tie, newer `submission_id` values are evicted first.

Recent public PRs fixed txpool DoS issues around vanity-address eviction,
expiring-nonce pool limits, and subblock RPC channel flooding.

## Design Implications for Kora

Kora's `eth_sendRawTransaction` path can remain Ethereum-compatible while still
leaving room for chain-specific routing later. Tempo is a useful reference
because it keeps standard JSON-RPC submission but layers additional routing and
pool semantics behind the RPC method.

For Kora, the main design questions are:

- Whether proposer-targeted or lane-specific submission should be encoded in the
  transaction payload, request metadata, or a separate RPC method.
- Whether txpool visibility methods should be available on public RPCs or only
  dedicated/operator nodes.
- Whether queued vs pending accounting should be part of the stable txpool
  interface from the beginning.
- How to enforce per-sender and pool-wide limits before expensive validation.

## Sources

- Tempo RPC Reference: <https://docs.tempo.xyz/protocol/rpc>
- Tempo Transaction Spec: <https://docs.tempo.xyz/protocol/transactions/spec-tempo-transaction>
- Tempo Payment Lane Spec: <https://docs.tempo.xyz/protocol/blockspace/payment-lane-specification>
- Tempo source: `crates/primitives/src/transaction/tempo_transaction.rs`
- Tempo source: `crates/primitives/src/subblock.rs`
- Tempo source: `crates/transaction-pool/src/tt_2d_pool.rs`
- PR #780: <https://github.com/tempoxyz/tempo/pull/780>
- PR #1628: <https://github.com/tempoxyz/tempo/pull/1628>
- PR #2216: <https://github.com/tempoxyz/tempo/pull/2216>
- PR #2327: <https://github.com/tempoxyz/tempo/pull/2327>
- Commit #3053: <https://github.com/tempoxyz/tempo/commit/ad930592facaafe9b7e8d9f781b7bc7d0426a3a1>
