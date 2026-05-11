# Daeji EVM Load Test Results - May 2026

This report summarizes the live EVM load tests run against the Daeji devnet at
`http://65.109.61.210:8545` during the EVM conformance investigation.

## Target

- Chain: Daeji / Nunchi Chain devnet
- RPC: `http://65.109.61.210:8545`
- Chain ID: `1337` (`0x539`)
- Client: `kora/0.1.0`
- Funded sender: `0xEb1Ba7Fc58b3416361a0EE07d140c91410c0AA8c`

## Test Series

### 1. Basic Transfer Load

Goal: verify that the producer can drain normal queued transactions under modest
write load.

Result:

- concurrency: `40`
- submitted: `200`
- accepted: `200`
- mined: `200`
- failures: `0`
- submit elapsed: `1.569s`
- observed submit rate: `127.431 tx/s`
- receipt statuses: `200` with `0x1`

Classification: passed. This confirmed that the devnet could include queued
normal transfers under modest load.

### 2. Live Contract/RPC Harness

Goal: verify contract deployment, direct state-changing calls, contract-to-contract
calls, concurrent same-contract writes, and fee-mode behavior before heavier load.

Result:

- fresh live harness: `24/25` checks passed
- `Counter` contract deployed successfully
- direct `increment()` transactions mined successfully
- contract-to-contract `Caller.callIncrement(Counter)` mined and mutated target state
- `20/20` concurrent same-contract increments mined successfully
- legacy `gasPrice` transfer mined
- EIP-1559 transfer with `maxFeePerGas = 1 gwei` and zero tip mined
- zero-fee EIP-1559 transfer was accepted and mined

Remaining failure:

- `eth_estimateGas` for `Counter.increment()` failed with `NonceTooLow`, suggesting
  estimate handling used nonce `0` instead of the sender's current state nonce.

Classification: mostly passed, with a remaining JSON-RPC estimate-gas issue and
non-mainnet-like devnet fee behavior that needs an explicit fee-model spec.

### 3. Massive Contract Stress

Goal: deploy several thousand contracts and execute several thousand state-changing
contract calls.

Harness shape:

- deploy target: `2,053` `StressCounter` contracts
- call target: `5,000` `increment()` calls
- bounded batches after initial nonce-gap rejection behavior

Result:

- initial raw burst of `2,000` deploys produced `nonce gap` rejections after the
  mempool accepted only a bounded future-nonce window
- after switching to bounded batches, the test completed the intended scale:
  `2,053` total deployed contracts and `5,000` state-changing calls

Classification: passed with bounded, nonce-aware submission. The initial failure
showed that the mempool enforces a future-nonce admission window, so high-volume
single-sender tests must avoid raw unbounded nonce bursts.

### 4. 10x Stress

Goal: scale the massive stress shape by roughly 10x.

Harness shape:

- deploy target: `20,530` contracts
- call target: `50,000` state-changing calls
- adaptive bounded batches

Result:

- deployed `13,094` contracts before the run stalled
- subsequent current-sender transactions were accepted by RPC and visible by hash
- receipts stayed `null`
- sender nonce stopped advancing
- blocks continued advancing

Follow-up probes:

- higher-gas same-nonce replacements were accepted and visible by hash, but did not mine
- previously funded auxiliary accounts also accepted current-nonce transfers that did
  not receive receipts during that run

Classification: failed. This exposed an accepted-but-unmined mempool-to-block-builder
inclusion stall under sustained load.

### 5. Big Stress Rerun

Goal: rerun the bounded massive stress shape after the devnet had reset/recovered.

Harness shape:

- deploy target: `2,053` contracts
- call target: `5,000` calls
- batch size: `250`
- workers: `64`

Result:

- preflight smoke transfer mined in about `2.5s`
- deploy batch 1: `250/250` confirmed
- deploy batch 2: `250/250` confirmed
- deploy batch 3: `249` submitted, `64/249` confirmed, `185` pending
- first call batch: `250/250` submitted, `0/250` confirmed
- total submitted: `999`
- total confirmed: `564`
- blocks advanced by `2,741` during the run
- final sender nonce matched confirmed transactions, not accepted submissions

Classification: failed. This reproduced the inclusion stall at a smaller scale.

### 6. Gap-Aware Rerun

The big stress rerun had one RPC read timeout during deploy batch 3, which left
room for a test-harness nonce-gap hypothesis. The harness in
`scripts/evm_load/gapaware_stress.py` was added to remove that ambiguity.

Harness fixes:

- computes each transaction hash locally before submission
- treats RPC send timeouts as unknown instead of failed
- polls `eth_getTransactionByHash` for unknown hashes before advancing
- retries the exact same signed transaction when a timed-out submission is not visible
- never proceeds to phase 2 while phase 1 has pending transactions from the same sender
- submits a same-nonce simple transfer backfill on stall and records whether it mines

Result from `2026-05-07`:

- starting head: `0x40d8d`
- starting latest nonce: `0x235`
- starting pending nonce: `0x235`
- deploy batch 1: `250/250` accepted starting at nonce `0x235`
- deploy receipts after about `360s`: `0/250`
- backfill transfer at nonce `0x235` with `50 gwei` gas price was accepted and
  visible by hash
- backfill receipt after about `180s`: `null`
- latest nonce after backfill: `0x235`
- pending nonce after backfill: `0x235`

Classification: failed. The gap-aware harness did not advance into the call phase.
It showed that current-nonce transactions can be accepted and visible by hash while
still not being included in advancing blocks.

## Conclusion

The load-test series shows two distinct behaviors:

- Daeji can mine modest transfer load and contract interaction load.
- Under sustained single-sender contract deployment/call stress, the devnet can enter
  a state where current-nonce transactions are accepted by RPC and visible by hash,
  but do not receive receipts while blocks continue advancing.

The gap-aware rerun removes the test-harness nonce-gap ambiguity for the latest
failure. The harness stops rather than manufacturing a larger future-nonce queue,
then records a same-nonce backfill that also fails to mine.

## Reproduction

Run against a local devnet or disposable remote devnet only:

```sh
python3 scripts/evm_load/gapaware_stress.py \
  --rpc-url http://127.0.0.1:8545 \
  --chain-id 1337 \
  --private-key-file /path/to/funded-key.txt \
  --out /tmp/daeji-gapaware-stress.json
```

The script requires:

- `python3`
- `requests`
- `eth-account`
- `eth-utils`
- `solc` on `PATH`

Do not run this against production or staging networks. It intentionally submits
large volumes of contract-creation and state-changing transactions.
