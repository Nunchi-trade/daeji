# Consensus Findings — Railway Validator Deployment

Observed on Railway (Nunchi Trade workspace, PRO plan, 24 vCPU / 24 GB).
The initial incident was on a single-validator simplex deployment with
`block_time_ms=50`. The current Railway deployment runs 3 validators in one
container with threshold 2.

---

## 1. Block Timestamps Are Block Height, Not Unix Time

**Previous location:** `crates/node/runner/src/runner.rs:85`,
`crates/node/runner/src/app.rs:71`

```rust
let header = Header {
    number: height,
    timestamp: height, // <-- block height, not wall clock
    ...
};
```

Previously, both `RevmContextProvider::context()` and `App::block_context()` set
`timestamp: height`. The EVM `TIMESTAMP` opcode returns this value, so
Solidity `block.timestamp` returns the block number.

**Why the naive wall-clock fix was unsafe:**
Block context is computed independently by the proposer (in `build_block`)
and the verifier (in `verify_block`). Both call `block_context()`. If you
use `SystemTime::now()`, the verifier computes a different timestamp than
the proposer → different EVM execution → different state root → verification
fails → consensus nullifies the round → **chain halts**.

Attempted `SystemTime::now()` fix caused the chain to halt after ~2000–3400
blocks every restart.

**Fix applied 2026-05-08:** Added `timestamp: u64` to `kora_domain::Block`
and included it in block encoding/decoding and the block digest. The proposer
chooses the timestamp once with `max(parent.timestamp + 1, unix_now_secs)`.
Verifiers and the finalized RPC context now read `block.timestamp`, so EVM
execution remains deterministic without equating timestamp to height.

This is a consensus format/digest break. The Railway auto config path now
regenerates validator material/genesis when the validator set changes, so the
deployed 3-validator chain is a fresh chain.

---

## 2. "block building failed" — Consensus Outruns the State Pipeline

**Log message (DEBUG, from commonware):**
```
commonware_consensus::marshal::standard::inline: skipping proposal
  parent_digest=... reason="block building failed"
```

**Immediate root cause:** `App::build_block()` at line 89:
```rust
let parent_snapshot = self.ledger.parent_snapshot(parent_digest).await?;
```
Returns `None` when the parent snapshot has not been inserted into the
in-memory snapshot cache yet.

Important correction: this is not only durable persistence lag. `build_block()`
executes the candidate block and computes its state root, but it does not cache
the producer-side snapshot before returning the block. Snapshot insertion
happens in `verify_block()` and in `FinalizedReporter` replay. If simplex asks
the app to build on top of a locally proposed-but-not-yet-verified/cached
parent, the cache lookup can fail even though the producer already computed the
state transition.

**Pattern observed:** The chain produces blocks in bursts:
- ~200–400 blocks built in ~2–5 seconds
- Interspersed with runs of "block building failed" where consensus advances
  views but can't build a block (parent snapshot missing)
- Nullified rounds fill the gap until the snapshot pipeline catches up

The view number advances roughly 2× the block height, meaning ~50% of
consensus rounds are nullified (no block proposed because building failed).

**Impact:** Usually recoverable in short bursts, but not guaranteed harmless.
When the cache path falls permanently behind or a proposal gets stuck after a
finalized parent, consensus can sit on nullifications without advancing height.
Even when it recovers, it wastes CPU cycling through nullified views and the
effective throughput is lower than what the local executor can deliver.

**Fix applied 2026-05-08:** `RevmApplication::build_block()` now inserts the
producer-side snapshot immediately after executing the proposed block, using
the same `OverlayState` merge pattern already used by the e2e harness. This
allows the next local proposal to build on a locally produced parent before
finalized replay catches up.

**Metrics from Railway deployment (observed 2026-05-08):**
- Measured block rate: ~76 blocks/sec (~11.4ms per finalized block)
- View-to-height ratio: ~2:1 (e.g., view 6866 at height 3433)
- The configured `block_time_ms=50` has no effect in single-validator mode;
  `leader_timeout` fires instantly because the sole leader proposes
  immediately

**Live re-check (2026-05-08 08:37 Europe/Berlin):**
- Railway project `kora`, environment `production`, service `kora`.
- Deployment `358b082b-3c05-4f62-b73b-35631f6c2dea` is `SUCCESS` and online at
  `https://kora-production-e104.up.railway.app`.
- Startup logs confirm `validators=1`, `secondary_peers=0`.
- RPC `eth_blockNumber` returned `0x169` (height 361).
- RPC `eth_getBlockByNumber("latest")` returned `number=0x169` and
  `timestamp=0x169`, confirming timestamp equals height on the live chain.
- RPC `kora_nodeStatus` stayed at `finalizedCount=361`, `currentView=725`
  across a 10 second sample while `nullifiedCount` rose from 30821 to 32300
  and later to 33923. This deployment was not merely losing half of views at
  that moment; height was stalled while nullifications continued.
- Last app logs show `built block` / `propose complete` for height 362 at
  `06:31:48Z`, then `broadcasting nullification floor` for view 722 whose
  payload is the finalized height-361 digest. RPC lookup for block `0x16a`
  returned `null`, so height 362 was proposed but not finalized/indexed.
- Current info-level logs do not include the debug-only
  `"block building failed"` message. Railway filters for `block building
  failed`, `missing parent snapshot`, `state root mismatch`, and
  `execution failed` returned no matches for the sampled deployment window.

**Post-fix Railway verification (2026-05-08 08:52 Europe/Berlin):**
- Deployed `d07f88d4-b478-4ccc-b192-55c9fdbd736e` with message
  `Fix consensus producer snapshot caching`.
- Railway reported deployment status `SUCCESS`.
- Fresh RPC check at uptime 15s returned `finalizedCount=454`,
  `proposedCount=460`, `nullifiedCount=5`, and `eth_blockNumber=0x1c7`.
- Follow-up check at uptime 36s returned `finalizedCount=917`,
  `proposedCount=947`, `nullifiedCount=31`, and `eth_blockNumber=0x39c`.
- Later check at uptime 78s returned `finalizedCount=1818`,
  `proposedCount=1913`, `nullifiedCount=101`, and `eth_blockNumber=0x737`.
- The live chain is advancing after redeploy. Occasional
  `dropped our proposal` warnings still appear, but they are no longer paired
  with a height stall in the sampled window.

**Final Railway verification after multi-validator deploy (2026-05-08):**
- Deployment `ebc76f95-03f4-4f4b-a4db-bcf919eb7eed` is `SUCCESS`.
- Railway variables: `NUM_VALIDATORS=3`, `THRESHOLD=2`,
  `BLOCK_TIME_MS=50`, `CHAIN_ID=1337`.
- Startup logs show `Starting 3 validators in this container`, validators
  0/1/2 loading DKG shares 0/1/2, and each runner registering
  `validators=3`.
- RPC sample at uptime 59s:
  `validatorCount=3`, `peerCount=2`, `finalizedCount=512`,
  `proposedCount=173`, `nullifiedCount=5`, `net_peerCount=0x2`.
- Follow-up sample 10s later:
  `finalizedCount=625`, `eth_blockNumber=0x276`.
- Latest block sample: `number=0x20b`, `timestamp=0x69fd9101`.
  Timestamp is no longer equal to height.
- Railway filters for `block building failed`, `missing parent snapshot`,
  `@level:error`, and `@level:warn` returned no matches for the sampled
  final deployment window.

---

## 3. "broadcasting nullification floor"

**Log message (WARN, from commonware):**
```
commonware_consensus::simplex::actors::voter::actor:
  broadcasting nullification floor
  floor=Finalization(Finalization { proposal: Proposal { round: Round { epoch: Epoch(0), view: View(N) }, ... } })
```

This fires when the voter detects it has advanced beyond a round without
finalizing it. In single-node mode it happens after every burst of block
building — the views that couldn't build a block (because of missing parent
snapshots) get nullified in bulk.

**Impact:** Harmless in single-node mode. Would indicate liveness problems
in multi-validator mode.

---

## 4. "dropped our proposal"

**Log message (WARN, from commonware):**
```
commonware_consensus::simplex::actors::voter::actor:
  dropped our proposal
  round=Round { epoch: Epoch(0), view: View(N) }
```

Seen occasionally. The voter proposed a block but the proposal was superseded
by a nullification before it could be finalized.

---

## 5. Buffer Capacity Warnings

**Log message (WARN, from commonware_runtime):**
```
commonware_runtime::utils::buffer::paged::append:
  requested buffer capacity is too low, increasing it to floor
  floor=131070
```

Also seen with `floor=2048`.

**Source:** Internal to `commonware_runtime` v2026.4.0 paged buffer allocator.
When a buffer allocation is requested below the minimum page size, the runtime
auto-corrects to the floor value and logs a warning.

**Configuration in runner (crates/node/runner/src/runner.rs ~line 440):**
```rust
simplex::Config {
    replay_buffer: NZUsize!(16 * 1024 * 1024),  // 16 MiB
    write_buffer: NZUsize!(16 * 1024 * 1024),   // 16 MiB
    ...
}
```

**Impact:** Completely harmless — the runtime self-corrects. The warnings are
just noisy. Suppressed via:
```
RUST_LOG=info,commonware_runtime::utils::buffer::paged::append=error
```

---

## 6. Railway Log Rate Limiting

**Log message:**
```
Railway rate limit reached for deployment, update your application to
reduce the logging rate. Messages dropped: 258
```

Railway enforces a log ingestion rate limit. At `block_time_ms=50` in
single-validator mode, the chain produces ~76 blocks/sec. Each block logs
at least 2 INFO lines ("built block" + "propose complete"). With debug
logging or buffer warnings enabled, this easily exceeds Railway's limit.

**Mitigation:**
- Use `RUST_LOG=info,commonware_runtime::utils::buffer=error` (no debug)
- Consider changing block build/propose logs to `debug!` or adding a
  sampling interval (log every Nth block)

---

## 7. `block_time_ms` Does Not Pace Single-Validator Blocks

The `block_time_ms` config (default 2000ms, set to 50ms in deployment) maps
to `leader_timeout` and `certification_timeout` in simplex consensus:

```rust
// crates/node/runner/src/runner.rs ~line 440
simplex::Config {
    leader_timeout: Duration::from_millis(self.block_time_ms),
    certification_timeout: Duration::from_millis(self.block_time_ms * 2),
    ...
}
```

In single-validator mode, the sole validator is always the leader and always
certifies its own proposals. The timeout is a *maximum* wait — if the leader
can propose immediately, it does. So blocks are produced as fast as execution
allows (~11ms), not at the configured interval.

**Fix applied 2026-05-08:** `RevmApplication::propose()` now sleeps via the
consensus environment before building a block. Simplex timeouts were widened
relative to the configured block time so a 50ms pacing delay does not collide
with a 50ms leader timeout. Live logs now show `propose complete` totals around
50-51ms.

---

## 8. Multi-Validator / Node Status

The Railway `auto` entrypoint now supports `NUM_VALIDATORS>1` by generating
per-node configs, patching localhost bootstrappers, assigning unique P2P/RPC
ports, and starting all validators in the same container. Node 0 exposes the
public JSON-RPC port; validators 1 and 2 bind local-only RPC ports.

`kora_nodeStatus` now includes `validatorCount` and computes `isLeader` from
the actual validator count. The simplex elector was changed to round-robin for
epoch 0 so the status fallback matches the deployed leader schedule. The RPC
server now seeds `kora_nodeStatus.peerCount` from the same configured peer
count used by `net_peerCount`.

---

## Summary Table

| Issue | Severity | Status | Fix Complexity |
|-------|----------|--------|----------------|
| Timestamps = block height | Medium | Fixed/deployed via `Block.timestamp` | Done |
| "block building failed" | Medium | Fixed/deployed for producer path | Follow-up tests/pruning |
| Nullification floor warns | Low | Not present in final sampled window | Monitor |
| Dropped proposals | Low | Not present in final sampled window | Monitor |
| Buffer capacity warns | Noise | Suppressed via RUST_LOG | None needed |
| Log rate limiting | Ops | Mitigated via RUST_LOG | Low (reduce log verbosity) |
| block_time_ms not pacing | Design | Fixed/deployed with app-level sleep | Done |
| Node status `isLeader` | Observability | Fixed for current round-robin elector | Done |
| Multi-validator Railway | High | Fixed/deployed with 3 validators / threshold 2 | Done |
