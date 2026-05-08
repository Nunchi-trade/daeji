# Consensus & RPC Fixes — Implementation Plan

Nine issues surfaced during the Railway deployment assessment. This plan groups
them into five workstreams that can be landed independently but share a
deliberate ordering: **each later workstream assumes the invariants established
by the earlier ones**.

## Issue Inventory

| # | Issue | Severity | Workstream |
|---|-------|----------|------------|
| 1 | Genesis block returns `null` from `eth_getBlockByNumber("0x0")` | Critical | [01-genesis](./01-genesis-indexing.md) |
| 2 | `eth_getBalance` / `eth_getTransactionCount` error on unknown accounts | Critical | [02-account-defaults](./02-account-defaults.md) |
| 3 | `baseFeePerGas` is `0x0` in blocks but `1 gwei` in `eth_feeHistory` | High | [03-base-fee](./03-base-fee-consistency.md) |
| 4 | `eth_getBlockReceipts` not implemented | High | [04-block-receipts](./04-block-receipts.md) |
| 5 | No WebSocket transport / `eth_subscribe` / `kora_subscribe` | High | [05-subscriptions](./05-subscriptions.md) |
| 6 | `hdc_permute` not exposed via RPC | Medium | [06-hdc-permute](./06-hdc-permute.md) |
| 7 | `hdc_cosineSimilarity` returns Method not found | Medium | [06-hdc-permute](./06-hdc-permute.md) |
| 8 | Chain ID config says 13370, node reports 1337 | Low | [07-config-hygiene](./07-config-hygiene.md) |
| 9 | `eth_getBlockReceipts` not implemented | — | Duplicate of #4 |

## Workstream Order

```
01-genesis-indexing        (no deps)
02-account-defaults        (no deps)
03-base-fee-consistency    (no deps)
04-block-receipts          (no deps)
05-subscriptions           (depends on 01 conceptually — WS newHeads
                            should emit genesis-consistent blocks)
06-hdc-permute             (no deps)
07-config-hygiene          (no deps)
```

Workstreams 01–04 and 06–07 are independent and can be developed in parallel.
05 (subscriptions) is the largest workstream and has its own phased plan.

## Guiding Principles

1. **Follow Ethereum JSON-RPC spec** — where the spec defines behavior for
   edge cases (unknown accounts, genesis block, baseFee), match it exactly.
   Don't invent kora-specific semantics unless there's a kora-specific method.

2. **Extend, don't fork** — add to existing traits and impls rather than
   creating parallel code paths. The `StateProvider` trait, `KoraApi` trait,
   and `HdcRpcApi` trait are the extension points.

3. **Feature-gate the big change** — WebSocket support is a new transport, not
   a patch. It gets its own Cargo feature (`ws`) on `kora-rpc` and a new
   `jsonrpsee` feature (`server-ws`).

4. **Test at the right layer** — each fix includes tests at the layer where
   the fix lives (state provider, RPC trait impl, integration).

## Files Involved (Summary)

| Crate | Key Files | Workstreams |
|-------|-----------|-------------|
| `kora-rpc` | `server.rs`, `eth.rs`, `kora.rs`, `hdc.rs`, `indexed_provider.rs`, `state_provider.rs`, `config.rs`, `error.rs`, `types.rs` | 01–07 |
| `kora-rpc` Cargo.toml | `Cargo.toml` | 05, 06 |
| `kora-indexer` | `lib.rs` | 01, 04 |
| `kora-hdc-chain` | `rpc.rs` | 06 |
| `kora-hdc` (core) | `vector.rs` | 06 |
| `kora-reporters` | `lib.rs` | 05 |
| `kora-runner` | `runner.rs` | 01, 05 |
| `kora-traits` | `lib.rs` | 02 |
| `kora-executor` | `revm.rs` | 03 |
| `deploy/railway/` | `init-config.env` | 07 |
