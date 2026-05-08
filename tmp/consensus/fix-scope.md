# Consensus Fix Scope and Deployment Record

Investigation date: 2026-05-08.

This started as a diagnosis and implementation scope note. After the Railway
stall and single-validator deployment were confirmed, the consensus/runtime
fixes were applied and deployed.

## Railway Snapshot

Commands used:

```sh
railway status
railway service status
railway deployment list
railway logs --lines 100 --json
curl -sS -X POST https://kora-production-e104.up.railway.app \
  -H 'content-type: application/json' \
  -d '{"jsonrpc":"2.0","method":"kora_nodeStatus","params":[],"id":1}'
curl -sS -X POST https://kora-production-e104.up.railway.app \
  -H 'content-type: application/json' \
  -d '{"jsonrpc":"2.0","method":"eth_getBlockByNumber","params":["latest",false],"id":2}'
```

Observed:

- Project: `kora`
- Environment: `production`
- Service: `kora`
- Deployment: `358b082b-3c05-4f62-b73b-35631f6c2dea`
- Public RPC URL: `https://kora-production-e104.up.railway.app`
- Region: `europe-west4-drams3a`
- Validator count from startup log: `1`
- Latest finalized RPC block during sampling: `0x169` / 361
- Latest block timestamp: `0x169`, equal to height
- Height stayed at 361 for at least a 10 second sample
- `nullifiedCount` continued increasing during that sample

## Applied Fix

Changed `crates/node/runner/src/app.rs` so
`RevmApplication::build_block()` caches the producer-side execution snapshot
before returning a block to simplex.

Implementation:

1. Execute the proposed block as before.
2. Build the new `OverlayState` with
   `parent_snapshot.state.merge_changes(outcome.changes.clone())`.
3. Insert the snapshot with
   `ledger.insert_snapshot(block_digest, parent_digest, next_state,
   state_root, outcome.changes, &block.txs).await`.
4. Leave `verify_block()` snapshot insertion in place for blocks received from
   other validators.

Local verification:

```sh
cargo check -p kora-runner --all-targets
```

Result: passed.

Railway deployment:

- Deployment: `d07f88d4-b478-4ccc-b192-55c9fdbd736e`
- Message: `Fix consensus producer snapshot caching`
- Status: `SUCCESS`
- Public RPC URL: `https://kora-production-e104.up.railway.app`

Post-fix live RPC samples:

- Uptime 15s: `finalizedCount=454`, `proposedCount=460`,
  `nullifiedCount=5`, `eth_blockNumber=0x1c7`.
- Uptime 36s: `finalizedCount=917`, `proposedCount=947`,
  `nullifiedCount=31`, `eth_blockNumber=0x39c`.
- Uptime 78s: `finalizedCount=1818`, `proposedCount=1913`,
  `nullifiedCount=101`, `eth_blockNumber=0x737`.

Conclusion: the live chain is operating after the redeploy. The pre-fix
deployment was stalled at height 361 with nullifications increasing; the
post-fix deployment advanced past height 1800 within the first 80 seconds.

## Final Railway Deployment

Final deployment:

- Deployment: `ebc76f95-03f4-4f4b-a4db-bcf919eb7eed`
- Message: `Expose three-validator status and peer count`
- Status: `SUCCESS`
- Public RPC URL: `https://kora-production-e104.up.railway.app`

Final Railway variables:

- `NUM_VALIDATORS=3`
- `THRESHOLD=2`
- `BLOCK_TIME_MS=50`
- `CHAIN_ID=1337`
- `BASE_P2P_PORT=30303`
- `BASE_RPC_PORT=8545`
- `RUST_LOG=info,commonware_runtime::utils::buffer=error`

Startup evidence from deployment `ebc76f95-03f4-4f4b-a4db-bcf919eb7eed`:

- Entrypoint logged `Starting 3 validators in this container`.
- Validators 0, 1, and 2 started with `/shared/kora-node0.toml`,
  `/shared/kora-node1.toml`, and `/shared/kora-node2.toml`.
- DKG output loaded with share indexes 0, 1, and 2.
- Each runner logged `Registered primary and secondary peers with oracle`
  with `validators=3`.

Final live RPC samples:

- Uptime 59s: `validatorCount=3`, `peerCount=2`, `finalizedCount=512`,
  `proposedCount=173`, `nullifiedCount=5`, `net_peerCount=0x2`.
- Latest block sample: `number=0x20b`, `timestamp=0x69fd9101`; timestamp is
  not height.
- Uptime 69s: `finalizedCount=625`, `eth_blockNumber=0x276`; height advanced
  during the 10 second sample.

Recent final-deployment Railway filters for `block building failed`,
`missing parent snapshot`, `@level:error`, and `@level:warn` returned no
matches.

## 1. Timestamp Fix

Previous behavior was deterministic because every execution context derived
`timestamp` from `height`:

- `crates/node/runner/src/app.rs`
- `crates/node/runner/src/runner.rs`
- `crates/node/consensus/src/proposal.rs`
- ledger tests in `crates/node/ledger/src/lib.rs`

Do not replace this with `SystemTime::now()` inside context construction. The
proposer and verifier independently execute the block; wall-clock reads will
diverge and produce different state roots.

Implemented fix:

1. Add `timestamp: u64` to `kora_domain::Block`.
2. Include it in `Block::write`, `EncodeSize`, `Read::read_cfg`, `id()`, and
   tests.
3. Update all `Block` constructors: genesis, tests, e2e harness, proposal
   builder, runner app.
4. Make proposer choose the timestamp once with
   `max(parent_timestamp + 1, unix_now_secs)`.
5. Make verifier and finalized RPC context read `block.timestamp` instead of
   recomputing from local time.
6. Use a hard codec/digest break for the current Railway reset.

Status: done and deployed. Risk was consensus-critical wire/digest change, so
the Railway chain was reset.

## 2. Parent Snapshot / Proposal Stall Fix

The fix was to cache the producer-side execution result inside
`RevmApplication::build_block()`.

Previous sequence:

1. `build_block()` loads parent snapshot.
2. It executes txs and computes the next state root.
3. It returns `Block`.
4. It does not insert the new snapshot.
5. Snapshot insertion happens later in `verify_block()` or finalized replay.

That leaves a gap where consensus can ask the app to build on a parent whose
execution was already computed locally but whose snapshot has not been cached.
Then `parent_snapshot(parent_digest)` returns `None` and commonware reports
`reason="block building failed"` at debug level.

Implemented fix:

1. In `build_block()`, after creating `block` and `block_digest`, compute:
   `merged_changes = parent_snapshot.state.merge_changes(outcome.changes.clone())`
   and `next_state = OverlayState::new(parent_snapshot.state.base(), merged_changes)`.
2. Call `ledger.insert_snapshot(block_digest, parent_digest, next_state,
   state_root, outcome.changes, &block.txs).await` before returning `Some(block)`.
3. Keep `verify_block()` insertion for proposals received from other validators.
4. Add tests that build two sequential local proposals without finalized replay
   and assert the second proposal can find the first proposal's snapshot.
5. Add a bounded-memory/pruning follow-up for unfinalized branch snapshots.

Status: done and deployed. Remaining follow-up is bounded-memory/pruning for
unfinalized branch snapshots and a focused regression test for sequential local
proposals.

Alternative fix:

- Make `propose()` wait briefly for a missing parent snapshot instead of
  immediately returning `None`.
- This is safer for memory but masks the producer cache gap and can block the
  automaton on a pipeline delay.

## 3. Actual Block Pacing

`block_time_ms` currently configures simplex timeouts:

- `leader_timeout`
- `certification_timeout`
- `timeout_retry`
- `fetch_timeout`

It is not a minimum block interval. In a 1-validator deployment the validator
can propose immediately, so observed blocks are much faster than 50 ms until
the snapshot/cache path stalls.

Implemented fix:

1. Add `block_time_ms` or `min_block_interval` to `RevmApplication`.
2. Pass the execution config value from `ProductionRunner`.
3. In `Application::propose`, use the provided `Env: Clock` and await
   `env.sleep(Duration::from_millis(min_block_interval))` or a deadline-based
   sleep before building/returning a block.
4. Widen simplex leader/certification/fetch timeouts relative to
   `block_time_ms` so the pacing sleep is not racing the consensus timeout.

Status: done and deployed. Live logs show `propose complete` around 50-51ms.
This is not a substitute for the snapshot cache fix; both are now in place.

## 4. Nullification / Dropped Proposal Warnings

These are mostly downstream of proposal/cache timing in single-validator mode.
The live deployment showed:

- `dropped our proposal` at view 605
- `broadcasting nullification floor` for view 722
- height 362 proposed twice but not finalized

Fixing producer snapshot caching should reduce these. If warnings remain after
that, run with targeted debug logs for:

```sh
commonware_consensus::marshal::standard::inline=debug
kora_runner::app=debug
kora_reporters=debug
```

Do not enable broad debug logging on Railway for long; it will hit log rate
limits quickly.

## 5. Log Volume

The app logs both `built block` and `propose complete` at `info!` for every
block. At 50-100 blocks/sec this alone can exceed Railway's log budget.

Candidate fix:

1. Move per-block `built block`, `propose complete`, `verified block`, and
   `verify complete` logs to `debug!`, or sample them every N blocks.
2. Keep startup, fatal execution errors, state-root mismatches, and operational
   milestones at `info!`/`warn!`.
3. Keep the Railway `RUST_LOG` suppression for commonware buffer warnings.

Complexity: low. Risk: low.

## 6. Buffer Capacity Warnings

No Kora code fix is required unless commonware exposes a config knob for the
specific internal page allocation. The runtime auto-corrects to its floor.

Operational fix:

```sh
RUST_LOG=info,commonware_runtime::utils::buffer::paged::append=error
```

## 7. Node Status Accuracy

`NodeState::set_view()` previously hardcoded leader calculation as `view % 4`.
The live Railway deployment had one validator, but `kora_nodeStatus` reported
`isLeader=false` at view 725. That was observability-only but misleading during
incident triage.

Implemented fix:

1. Store validator count in `NodeState`.
2. Compute `is_leader` with that count.
3. Use the same round-robin leader schedule in simplex for the current epoch 0
   deployment.
4. Seed `kora_nodeStatus.peerCount` from the same configured peer count used by
   `net_peerCount`.

Status: done and deployed. Final RPC sample reported `validatorCount=3`,
`peerCount=2`, and `net_peerCount=0x2`.

## 8. Multi-Validator Docker/Railway Mode

Implemented in `docker/scripts/entrypoint.sh`:

1. `auto` mode honors `NUM_VALIDATORS` and `THRESHOLD`.
2. Existing generated config is regenerated when validator count, threshold, or
   chain id no longer match.
3. Per-validator node config/data directories are created under `/shared` and
   `/data`.
4. Bootstrappers are patched from Docker service names to unique localhost P2P
   ports for single-container Railway.
5. Validator 0 exposes the public JSON-RPC port; validators 1 and 2 bind
   local-only RPC ports.
6. The entrypoint starts all validators and waits on the first process exit.

Status: done and deployed with 3 validators and threshold 2.

## Recommended Order

1. Cache producer snapshots in `build_block()` - done and deployed.
2. Add `Block.timestamp` as a consensus-format change - done and deployed.
3. Add real pacing in `propose()` - done and deployed.
4. Run Railway in 3-validator auto mode - done and deployed.
5. Fix `NodeState` leader and peer reporting - done and deployed.
6. Add targeted diagnostics/tests for missing parent snapshots - remaining.
7. Move noisy per-block logs to debug or sampling - remaining.
