# Daeji EVM Conformance Results - 2026-05-05

## Summary

This report captures the Daeji/Nunchi Chain EVM conformance work from 2026-05-05.

The live Daeji devnet at `http://65.109.61.210:8545` is able to mine normal transfers, deploy contracts, answer basic Ethereum JSON-RPC reads, execute contract-to-contract calls, and sustain concurrent same-contract writes. However, the canonical EVM fixture baseline from `~/evm-specpool-impl` is currently blocked because `specpool-evm` does not compile with the feature set required by its EEST runner.

Per the test plan, differential fuzzing should not proceed until the canonical baseline runner is made clean or each build/fork-line gap is intentionally classified.

## Target

- Chain: Daeji / Nunchi Chain
- RPC tested: `http://65.109.61.210:8545`
- Chain ID: `1337`
- Client: `kora/0.1.0`
- Deployer: `0xEb1Ba7Fc58b3416361a0EE07d140c91410c0AA8c`
- Execution repo reconned: `~/evm-specpool-impl`

## Recon Findings

`~/evm-specpool-impl` exists and exposes a top-level Cargo workspace containing `specpool-evm`.

No `revme` binary exists in the repo, so the requested command:

```bash
cargo run -p specpool-evm --bin revme -- statetest /tmp/eest-fixtures/state_tests/
```

cannot run as written.

The repo's practical fixture runner is `specpool-evm/src/testing/ef_runner.rs`, which defines ignored tests for EEST and Ethereum Foundation fixtures.

The highest explicit execution fork-line found in runnable EVM construction is Shanghai:

- `specpool-evm/src/execution/preexec.rs` uses `SpecId::SHANGHAI`
- `specpool-evm/src/benchmark/revm_executor.rs` uses `SpecId::SHANGHAI`
- `specpool-evm/src/integration/test_harness.rs` still uses `SpecId::LONDON`

Treat Shanghai as the conformance target until Cancun/Prague support is made explicit in the Daeji execution path.

## Canonical Baseline

Command rerun from `~/evm-specpool-impl` on 2026-05-05:

```bash
cargo test -p specpool-evm --features evm,alloy test_run_eest_shanghai -- --ignored --nocapture
```

Result: failed before fixture execution.

Representative compile blockers:

- `networking/sparrow.rs` imports `topics`, but `topics` is gated behind `p2p`.
- `execution/validator.rs` implements an older revm `DatabaseRef` API and is missing `basic_account`, `bytecode_by_hash`, and `block_hash`.
- Multiple stale primitive conversions exist across `kauri_application.rs`, `dag_validator.rs`, `env_classifier.rs`, `dag.rs`, and `dag_builder.rs`.

Classification: this is a build/harness failure, not an EEST semantic failure. The baseline is not clean, so differential fuzzing was intentionally not run.

## Fresh Live Harness

Command: fresh Python/Web3 harness against `http://65.109.61.210:8545` using the prefunded deployer key.

Result: 24/25 passed.

Fresh run timestamp: `2026-05-05T15:31:20Z`.

Passing surfaces:

- `eth_chainId` returned `0x539`.
- `web3_clientVersion` returned `kora/0.1.0`.
- `eth_blockNumber` advanced from `0x2e63` to `0x2e6d` over 3 seconds.
- latest block returned expected gas, base fee, and state-root fields.
- `eth_gasPrice` returned `0x3b9aca00`.
- `eth_feeHistory` returned `baseFeePerGas`.
- `eth_getCode` unknown returned `0x`.
- `eth_getStorageAt` unknown succeeded.
- `eth_getLogs` for latest succeeded.
- freshly compiled `Counter` contract deployed at `0xe71F20AA04616DA5Ff30D7CB2F5811125f7ea819`.
- `eth_getCode` for `Counter` returned 432 bytes.
- `Counter.get()` initially returned `0`.
- 3 direct `increment()` transactions mined successfully.
- `Counter.get()` and `eth_getStorageAt` slot 0 both reflected direct increments.
- freshly compiled `Caller` contract deployed at `0xE042a21F103b767bAd8F23fB7A65A5F1fB334EEC`.
- contract-to-contract `Caller.callIncrement(Counter)` mined and mutated `Counter` from 3 to 4.
- 20 concurrent same-contract increments submitted and mined, all with status `1`.
- `Counter` reflected all 20 concurrent increments, ending at 24.
- legacy `gasPrice` transfer mined.
- EIP-1559 transfer with `maxFeePerGas = 1 gwei` and zero tip mined.
- EIP-1559 zero-fee transfer was accepted and mined.

Fresh failure:

- `eth_estimateGas` for `Counter.increment()` failed with `NonceTooLow { tx: 0, state: 50 }`.

Classification: the earlier contract-to-contract mutation failure did not reproduce in this fresh run. The remaining reproducible live issue is `eth_estimateGas` using nonce `0` instead of the sender's current state nonce when estimating from the funded deployer.

## Live RPC Smoke

Result: covered again as part of the fresh 24/25 live harness against `http://65.109.61.210:8545`.

Covered:

- `eth_chainId`
- `web3_clientVersion`
- block advancement
- latest block fields
- `eth_gasPrice`
- `eth_feeHistory`
- `eth_getCode` for unknown and known contracts
- `eth_getStorageAt`
- `eth_getLogs`
- `eth_call`

Notable fee observation:

- `eth_gasPrice` returned `1 gwei`.
- `eth_feeHistory` returned `baseFeePerGas = 1 gwei`.
- latest block response showed `baseFeePerGas = 0`.

That disagreement should be treated as a fee-model conformance issue until clarified.

## Earlier Funded Live Harness

Earlier result: 19/23 passed against `http://65.109.61.210:8545`.

Passing surfaces:

- connection, chain ID, client version, and block production
- prefunded deployer balance
- funding auxiliary accounts
- contract deployment
- `eth_getCode` on deployed contract
- `eth_call` reads
- direct repeated state-changing calls
- same-contract concurrent transaction admission and mining
- legacy `gasPrice` transfer
- EIP-1559 transfer with 1 gwei max fee and zero tip
- observed zero-fee EIP-1559 transfer accepted and mined

Earlier failures:

- stock e2e script expects unfunded Hardhat account `0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266`
- contract-to-contract `CALL` returned receipt status `1` but did not mutate the target counter; this did not reproduce in the fresh run above
- final counter value after concurrent run was one increment short, consistent with the failed contract-to-contract mutation; this did not reproduce in the fresh run above
- `eth_estimateGas` for `increment()` failed with `NonceTooLow { tx: 0, state: 21 }`

## Load Test

Result: 200/200 accepted and mined.

Key metrics:

- concurrency: 40
- submitted: 200
- accepted: 200
- mined: 200
- failed: 0
- submit elapsed: 1.569 seconds
- submit rate: 127.431 tx/s
- receipt statuses: 200 with `0x1`

This confirms the producer-drain fix can include queued transactions under modest write load on the tested devnet.

## Fee-Mechanism Characterization

Observed behavior:

- legacy transfer mines with `gasPrice`
- EIP-1559 transfer mines with `maxFeePerGas = 1 gwei` and `maxPriorityFeePerGas = 0`
- EIP-1559 zero-fee transfer was accepted and mined in both the earlier and fresh live harnesses
- `eth_feeHistory` and latest block disagree on base fee

Classification:

The live endpoint does not appear to enforce mainnet-style EIP-1559 fee validity. This may be intentional for devnet, but it needs an explicit Daeji fee-model spec before these behaviors can be classified as expected.

## Devnet Stability

During earlier retries, sibling RPC ports on `65.109.61.210` were observed in non-advancing states. A blocked run recorded:

- literal `:210`: connection refused
- `:8545`: chain ID/client OK, head `0x0`, no drift
- `:8546`: head `0xfb258`, no drift
- `:8547`: head `0x1562`, no drift
- `:8548`: head `0xfb258`, no drift

Later `:8545` resumed block production and the funded harness ran there.

## Definition-of-Done Status

- Step 0 recon: complete and saved to `~/obsidian-vault/research/2026-05-05-daeji-recon.md`.
- Step 1 baseline: rerun, blocked by compile failure before fixtures.
- Step 2 differential: intentionally not run because Step 1 did not establish a clean baseline and no Daeji `revme t8n` binary exists.
- Step 3 fee stress: partial live characterization complete; zero-fee EIP-1559 behavior was reproed; no local exhaustive fee stress yet.
- Step 4 local devnet stress: not run in this PR. Local-only stress should use `just trusted-devnet` or `just devnet` from the current `daeji` repo, then run the heavier destructive workload locally only.

## Jacob-Ready Summary

Daeji's live devnet is now mining real contract and transfer traffic, including a 200/200 mined load test and a fresh 24/25 contract/RPC/fee harness. The JSON-RPC surface is much healthier than the earlier stalled runs. The important blockers are now conformance-specific: the `evm-specpool-impl` canonical fixture runner does not compile against its current revm/alloy dependencies, there is no `revme t8n` binary for goevmlab, `eth_estimateGas` still fails with `NonceTooLow`, and fee behavior is not mainnet EIP-1559 conformant unless zero-fee EIP-1559 mining is an intentional devnet rule.

Next action: make `specpool-evm --features evm,alloy` compile, run EEST Shanghai cleanly, then add a `revme t8n` binary or equivalent adapter so goevmlab can run at 10k+ programs against matching upstream revm.
