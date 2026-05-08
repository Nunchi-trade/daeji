# 07 -- Registering the HDC Precompile at Address 0x09 in the Kora REVM Executor

> **STATUS: DIVERGED -- PR #42 SUPERSEDES THIS DOCUMENT**
>
> This document describes registering an HDC precompile at address **0x09** with
> raw opcode-byte dispatch and gas costs of 1,500/500 per operation. This plan
> was never merged as written.
>
> **PR #42** (`kora-precompiles` crate) implements a different design:
> - Address **0xA0C** (not 0x09) for `HDCPrecompile`
> - Address **0xA0D** for `StigmergyPrecompile` (not in this doc at all)
> - **4-byte selectors** (function-signature style), not single opcode bytes
> - **50,000 gas** flat cost (not the 500/1,500 per-opcode schedule here)
> - **Event-replay consensus model** (precompile index rebuilt from chain events)
> - Located in `crates/precompiles/` (not `crates/node/executor/src/precompiles/`)
>
> **Follow PR #42's design for all new work.** This document is retained as
> historical context for the original plan only.
>
> **Last updated:** 2026-05-08

## Context

The daeji/kora executor uses **REVM 38.0.0** (handler v18.1.0, precompile crate v34.0.0) to
execute EVM blocks.  The entry point is `RevmExecutor` in
`crates/node/executor/src/revm.rs`.  Today the executor calls
`ctx.build_mainnet()` which constructs an `Evm` struct whose `precompiles`
field is an `EthPrecompiles` -- the stock set for the configured spec
(`SpecId::CANCUN`).

There are **no** custom precompiles today.  This document explains, step by
step, how to add one at address **0x09** that implements the HDC instruction
set.

### Address 0x09 conflict with BLAKE2F

Under CANCUN, address `0x09` is the BLAKE2 `F` compression function
(EIP-152).  Registering the HDC precompile at the same address **replaces**
BLAKE2F.  This is intentional -- daeji is a sovereign chain that does not
need BLAKE2F compatibility.  If you ever need both, pick a higher address
(e.g., `0x42`) and adjust the address constant below.

---

## Architecture

```
crates/node/executor/
  Cargo.toml                    # add kora-hdc dependency
  src/
    lib.rs                      # add `mod precompiles;`
    revm.rs                     # change build_mainnet -> custom Evm construction
    precompiles/
      mod.rs                    # re-export hdc module, define HdcPrecompiles provider
      hdc.rs                    # PrecompileFn implementation for 0x09
```

The key insight is that `build_mainnet()` returns
`Evm<CTX, (), EthInstructions, EthPrecompiles, EthFrame>`.  The
`EthPrecompiles` type holds a `&'static Precompiles` reference to an
immutable table.  You cannot mutate it.

There are two viable approaches to inject a custom precompile:

- **Approach A (recommended):** Write a custom `PrecompileProvider` that
  wraps `EthPrecompiles` and intercepts address `0x09`.
- **Approach B:** Stop using `build_mainnet()`, construct the `Evm` struct
  manually, and pass a custom `PrecompileProvider`.

Both approaches need the same `PrecompileProvider` implementation.
Approach A is cleaner because it avoids duplicating `build_mainnet` logic
and works if the REVM internals change between versions.

We use **Approach A** below.

---

## Step 1 -- Add dependency

Edit `crates/node/executor/Cargo.toml`:

```toml
[dependencies]
# ... existing deps ...
kora-hdc = { path = "../../hdc" }          # HDC vector operations crate
```

> If `kora-hdc` does not exist yet, create a stub crate that exposes the
> pure-Rust HDC primitives (hamming, bind, bundle, permute).  The precompile
> will call into it.

---

## Step 2 -- Create `crates/node/executor/src/precompiles/mod.rs`

```rust
//! Custom precompile providers for the Kora executor.

pub mod hdc;

use hdc::HdcPrecompiles;
pub use hdc::HDC_ADDRESS;
```

---

## Step 3 -- Create `crates/node/executor/src/precompiles/hdc.rs`

This is the core file.  It defines a `PrecompileProvider` wrapper that
intercepts calls to address `0x09` and delegates everything else to the
stock `EthPrecompiles`.

### 3a -- Constants and address

```rust
//! HDC precompile at address 0x09.
//!
//! Opcode map:
//!   0x01  hdc_hamming(a, b)       -- 2x1280 B input, 4 B output
//!   0x02  hdc_bind(a, b)          -- 2x1280 B input, 1280 B output
//!   0x03  hdc_bundle(vecs, count) -- count x 1280 B input, 1280 B output
//!   0x04  hdc_permute(v, n)       -- 1280 B + 4 B input, 1280 B output
//!   0x05  storeVector(key, vec)   -- 32 B + 1280 B input  (state write)
//!   0x06  searchSimilar(q, k)     -- 1280 B + 4 B input, k x 36 B output

use revm::precompile::{
    PrecompileFn, PrecompileHalt, PrecompileId, PrecompileOutput, PrecompileResult,
    PrecompileStatus,
};
use revm::primitives::{Address, Bytes};

/// The precompile lives at address 0x09, replacing BLAKE2F on this chain.
pub const HDC_ADDRESS: Address = Address::new([
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0x09,
]);

/// Hypervector dimensionality in bytes (10240 bits / 8).
const HDC_DIM_BYTES: usize = 1280;
```

### 3b -- The `PrecompileFn` entry point

The REVM precompile function signature (v34.0.0) is:

```rust
type PrecompileFn = fn(&[u8], u64, u64) -> PrecompileResult;
//                      input  gas_limit  reservoir
```

Implement it like this:

```rust
/// Top-level HDC precompile dispatcher.
///
/// Input layout:
///   byte 0:       opcode (0x01..0x06)
///   bytes 1..:    opcode-specific payload
pub fn hdc_precompile(input: &[u8], gas_limit: u64, reservoir: u64) -> PrecompileResult {
    if input.is_empty() {
        return Ok(PrecompileOutput::halt(
            PrecompileHalt::other_static("hdc: empty input"),
            reservoir,
        ));
    }

    let opcode = input[0];
    let payload = &input[1..];

    match opcode {
        0x01 => op_hamming(payload, gas_limit, reservoir),
        0x02 => op_bind(payload, gas_limit, reservoir),
        0x03 => op_bundle(payload, gas_limit, reservoir),
        0x04 => op_permute(payload, gas_limit, reservoir),
        0x05 => op_store_vector(payload, gas_limit, reservoir),
        0x06 => op_search_similar(payload, gas_limit, reservoir),
        _ => Ok(PrecompileOutput::halt(
            PrecompileHalt::other_static("hdc: unknown opcode"),
            reservoir,
        )),
    }
}
```

### 3c -- Opcode implementations

Each function validates input length, checks gas, performs the operation,
and returns `PrecompileOutput`.

```rust
// ── 0x01: hdc_hamming ───────────────────────────────────────────────
// Input:  2 x 1280 bytes
// Output: 4 bytes (u32 big-endian hamming distance)
// Gas:    1500

const GAS_HAMMING: u64 = 1_500;

fn op_hamming(payload: &[u8], gas_limit: u64, reservoir: u64) -> PrecompileResult {
    if payload.len() != 2 * HDC_DIM_BYTES {
        return Ok(PrecompileOutput::halt(
            PrecompileHalt::other_static("hdc_hamming: expected 2560 bytes"),
            reservoir,
        ));
    }
    if gas_limit < GAS_HAMMING {
        return Ok(PrecompileOutput::halt(PrecompileHalt::OutOfGas, reservoir));
    }

    let a = &payload[..HDC_DIM_BYTES];
    let b = &payload[HDC_DIM_BYTES..];

    // Compute hamming distance using integer popcount -- NO f64 ALLOWED.
    let distance: u32 = a
        .iter()
        .zip(b.iter())
        .map(|(x, y)| (x ^ y).count_ones())
        .sum();

    let output = Bytes::copy_from_slice(&distance.to_be_bytes());
    Ok(PrecompileOutput::new(GAS_HAMMING, output, reservoir))
}

// ── 0x02: hdc_bind ──────────────────────────────────────────────────
// Input:  2 x 1280 bytes
// Output: 1280 bytes (component-wise XOR)
// Gas:    500

const GAS_BIND: u64 = 500;

fn op_bind(payload: &[u8], gas_limit: u64, reservoir: u64) -> PrecompileResult {
    if payload.len() != 2 * HDC_DIM_BYTES {
        return Ok(PrecompileOutput::halt(
            PrecompileHalt::other_static("hdc_bind: expected 2560 bytes"),
            reservoir,
        ));
    }
    if gas_limit < GAS_BIND {
        return Ok(PrecompileOutput::halt(PrecompileHalt::OutOfGas, reservoir));
    }

    let a = &payload[..HDC_DIM_BYTES];
    let b = &payload[HDC_DIM_BYTES..];

    // Bind = XOR for binary hypervectors.
    let result: Vec<u8> = a.iter().zip(b.iter()).map(|(x, y)| x ^ y).collect();

    Ok(PrecompileOutput::new(
        GAS_BIND,
        Bytes::from(result),
        reservoir,
    ))
}

// ── 0x03: hdc_bundle ────────────────────────────────────────────────
// Input:  count x 1280 bytes  (count inferred from input length)
// Output: 1280 bytes (majority-vote bundle)
// Gas:    500 + 100 * count

const GAS_BUNDLE_BASE: u64 = 500;
const GAS_BUNDLE_PER_VEC: u64 = 100;

fn op_bundle(payload: &[u8], gas_limit: u64, reservoir: u64) -> PrecompileResult {
    if payload.is_empty() || payload.len() % HDC_DIM_BYTES != 0 {
        return Ok(PrecompileOutput::halt(
            PrecompileHalt::other_static("hdc_bundle: input must be N * 1280 bytes"),
            reservoir,
        ));
    }

    let count = payload.len() / HDC_DIM_BYTES;
    let gas_required = GAS_BUNDLE_BASE + GAS_BUNDLE_PER_VEC * (count as u64);

    if gas_limit < gas_required {
        return Ok(PrecompileOutput::halt(PrecompileHalt::OutOfGas, reservoir));
    }

    // Majority vote: for each bit position, count 1s across all vectors.
    // If count_ones > count/2, output bit is 1.  Ties (even count) break to 0.
    let threshold = count / 2;
    let mut result = vec![0u8; HDC_DIM_BYTES];

    // Accumulate per-bit counts using u16 counters (max count < 65535).
    let mut counts = vec![0u16; HDC_DIM_BYTES * 8];
    for i in 0..count {
        let vec_start = i * HDC_DIM_BYTES;
        let v = &payload[vec_start..vec_start + HDC_DIM_BYTES];
        for (byte_idx, &byte_val) in v.iter().enumerate() {
            for bit in 0..8u8 {
                if byte_val & (1 << (7 - bit)) != 0 {
                    counts[byte_idx * 8 + bit as usize] += 1;
                }
            }
        }
    }

    for (bit_idx, &c) in counts.iter().enumerate() {
        if (c as usize) > threshold {
            let byte_idx = bit_idx / 8;
            let bit_pos = 7 - (bit_idx % 8);
            result[byte_idx] |= 1 << bit_pos;
        }
    }

    Ok(PrecompileOutput::new(
        gas_required,
        Bytes::from(result),
        reservoir,
    ))
}

// ── 0x04: hdc_permute ───────────────────────────────────────────────
// Input:  1280 bytes (vector) + 4 bytes (n as u32 big-endian)
// Output: 1280 bytes (bit-rotated by n positions)
// Gas:    500

const GAS_PERMUTE: u64 = 500;

fn op_permute(payload: &[u8], gas_limit: u64, reservoir: u64) -> PrecompileResult {
    if payload.len() != HDC_DIM_BYTES + 4 {
        return Ok(PrecompileOutput::halt(
            PrecompileHalt::other_static("hdc_permute: expected 1284 bytes"),
            reservoir,
        ));
    }
    if gas_limit < GAS_PERMUTE {
        return Ok(PrecompileOutput::halt(PrecompileHalt::OutOfGas, reservoir));
    }

    let v = &payload[..HDC_DIM_BYTES];
    let n_bytes: [u8; 4] = payload[HDC_DIM_BYTES..HDC_DIM_BYTES + 4]
        .try_into()
        .unwrap();
    let n = u32::from_be_bytes(n_bytes) as usize;

    let total_bits = HDC_DIM_BYTES * 8;
    let shift = n % total_bits;

    if shift == 0 {
        return Ok(PrecompileOutput::new(
            GAS_PERMUTE,
            Bytes::copy_from_slice(v),
            reservoir,
        ));
    }

    // Circular left-rotate the entire bit-vector by `shift` bits.
    let mut result = vec![0u8; HDC_DIM_BYTES];
    for src_bit in 0..total_bits {
        let dst_bit = (src_bit + total_bits - shift) % total_bits;
        let src_byte = src_bit / 8;
        let src_offset = 7 - (src_bit % 8);
        let dst_byte = dst_bit / 8;
        let dst_offset = 7 - (dst_bit % 8);
        if v[src_byte] & (1 << src_offset) != 0 {
            result[dst_byte] |= 1 << dst_offset;
        }
    }

    Ok(PrecompileOutput::new(
        GAS_PERMUTE,
        Bytes::from(result),
        reservoir,
    ))
}

// ── 0x05: storeVector (stub) ────────────────────────────────────────
// Input:  32 bytes (key) + 1280 bytes (vector)
// Gas:    varies -- depends on SSTORE semantics
//
// This opcode requires STATE ACCESS.  A pure PrecompileFn cannot write
// state.  It needs either:
//   (a) a stateful PrecompileProvider that holds a &mut to the DB, or
//   (b) post-processing after the precompile returns.
//
// For now, return a halting error.  See "Stateful opcodes" section below.

fn op_store_vector(payload: &[u8], _gas_limit: u64, reservoir: u64) -> PrecompileResult {
    if payload.len() != 32 + HDC_DIM_BYTES {
        return Ok(PrecompileOutput::halt(
            PrecompileHalt::other_static("storeVector: expected 1312 bytes"),
            reservoir,
        ));
    }
    // TODO: implement via stateful PrecompileProvider (see section below)
    Ok(PrecompileOutput::halt(
        PrecompileHalt::other_static("storeVector: not yet implemented"),
        reservoir,
    ))
}

// ── 0x06: searchSimilar (stub) ──────────────────────────────────────
// Input:  1280 bytes (query) + 4 bytes (k as u32)
// Output: k x 36 bytes (32-byte key + 4-byte distance per result)
// Gas:    varies

fn op_search_similar(payload: &[u8], _gas_limit: u64, reservoir: u64) -> PrecompileResult {
    if payload.len() != HDC_DIM_BYTES + 4 {
        return Ok(PrecompileOutput::halt(
            PrecompileHalt::other_static("searchSimilar: expected 1284 bytes"),
            reservoir,
        ));
    }
    // TODO: implement via stateful PrecompileProvider (see section below)
    Ok(PrecompileOutput::halt(
        PrecompileHalt::other_static("searchSimilar: not yet implemented"),
        reservoir,
    ))
}
```

### 3d -- The `HdcPrecompiles` provider

This wraps `EthPrecompiles` and intercepts address `0x09`:

```rust
use revm::handler::{EthPrecompiles, PrecompileProvider, precompile_provider::precompile_output_to_interpreter_result};
use revm::precompile::{Precompile, PrecompileId, Precompiles, PrecompileSpecId};
use interpreter::{CallInputs, InterpreterResult};
use context::{Cfg, LocalContextTr};
use context_interface::{ContextTr, JournalTr};
use std::boxed::Box;

/// Custom precompile provider that adds the HDC precompile at 0x09
/// on top of the standard Ethereum precompile set.
#[derive(Debug, Clone)]
pub struct HdcPrecompiles {
    /// The underlying standard Ethereum precompiles.
    inner: EthPrecompiles,
}

impl HdcPrecompiles {
    /// Create a new HDC precompile provider for the given spec.
    pub fn new(spec: revm::primitives::hardfork::SpecId) -> Self {
        Self {
            inner: EthPrecompiles::new(spec),
        }
    }
}

impl<CTX: ContextTr> PrecompileProvider<CTX> for HdcPrecompiles {
    type Output = InterpreterResult;

    fn set_spec(&mut self, spec: <CTX::Cfg as Cfg>::Spec) -> bool {
        self.inner.set_spec(spec)
    }

    fn run(
        &mut self,
        context: &mut CTX,
        inputs: &CallInputs,
    ) -> Result<Option<InterpreterResult>, String> {
        // Intercept calls to 0x09
        if inputs.bytecode_address == HDC_ADDRESS {
            let input_bytes = inputs.input.as_bytes(context);
            let output = hdc_precompile(&input_bytes, inputs.gas_limit, inputs.reservoir)
                .map_err(|e| e.to_string())?;

            // Persist halt context for top-level calls, matching EthPrecompiles behavior.
            if let Some(halt_reason) = output.halt_reason() {
                if !halt_reason.is_oog() && context.journal().depth() == 1 {
                    context
                        .local_mut()
                        .set_precompile_error_context(halt_reason.to_string());
                }
            }

            let result = precompile_output_to_interpreter_result(output, inputs.gas_limit);
            return Ok(Some(result));
        }

        // Everything else falls through to standard precompiles.
        self.inner.run(context, inputs)
    }

    fn warm_addresses(&self) -> Box<impl Iterator<Item = Address>> {
        // Include 0x09 in the warm set along with all standard addresses.
        let std_addrs: Vec<Address> = self.inner.precompiles.addresses().cloned().collect();
        let mut addrs = std_addrs;
        if !addrs.contains(&HDC_ADDRESS) {
            addrs.push(HDC_ADDRESS);
        }
        Box::new(addrs.into_iter())
    }

    fn contains(&self, address: &Address) -> bool {
        *address == HDC_ADDRESS || self.inner.contains(address)
    }
}
```

---

## Step 4 -- Register the module in `crates/node/executor/src/lib.rs`

Add one line:

```rust
mod precompiles;
```

after the existing `mod revm;` line.

---

## Step 5 -- Modify `crates/node/executor/src/revm.rs`

The changes are minimal.  Replace `build_mainnet()` with manual `Evm`
construction that uses `HdcPrecompiles` instead of `EthPrecompiles`.

### 5a -- Add imports

At the top of `revm.rs`, add:

```rust
use revm::{
    // existing imports stay...
    handler::{EthInstructions, EthFrame},  // add these
};
use revm::interpreter::interpreter::EthInterpreter; // add this

use crate::precompiles::hdc::HdcPrecompiles;        // add this
```

### 5b -- Create a helper function

Add a helper that mirrors `build_mainnet()` but swaps in `HdcPrecompiles`:

```rust
use revm::context::{Evm as EvmStruct, FrameStack};

/// Build an EVM instance with the HDC precompile registered at 0x09.
///
/// This is equivalent to `ctx.build_mainnet()` but uses `HdcPrecompiles`
/// instead of `EthPrecompiles`.
fn build_evm_with_hdc<BLOCK, TX, CFG, DB, JOURNAL, CHAIN>(
    ctx: Context<BLOCK, TX, CFG, DB, JOURNAL, CHAIN>,
) -> EvmStruct<
    Context<BLOCK, TX, CFG, DB, JOURNAL, CHAIN>,
    (),
    EthInstructions<EthInterpreter, Context<BLOCK, TX, CFG, DB, JOURNAL, CHAIN>>,
    HdcPrecompiles,
    EthFrame<EthInterpreter>,
>
where
    BLOCK: revm::context_interface::Block,
    TX: revm::context_interface::Transaction,
    CFG: revm::context::Cfg,
    DB: revm::Database,
    JOURNAL: revm::context_interface::JournalTr<Database = DB>,
{
    let spec: revm::primitives::hardfork::SpecId = ctx.cfg.spec().into();
    EvmStruct {
        ctx,
        inspector: (),
        instruction: EthInstructions::new_mainnet_with_spec(spec),
        precompiles: HdcPrecompiles::new(spec),
        frame_stack: FrameStack::new_prealloc(8),
    }
}
```

### 5c -- Update `execute()` in `impl BlockExecutor`

In the `execute` method (line 383 of the current file), change:

```rust
// BEFORE:
let mut evm = ctx.build_mainnet();

// AFTER:
let mut evm = build_evm_with_hdc(ctx);
```

### 5d -- Update `simulate_call()`

Same change in `simulate_call()` (line 234 of the current file):

```rust
// BEFORE:
let mut evm = ctx.build_mainnet();

// AFTER:
let mut evm = build_evm_with_hdc(ctx);
```

### 5e -- Remove unused import

After these changes, `MainBuilder` is no longer used.  Remove it from the
`use revm::{...}` block to avoid a compiler warning.

---

## Step 6 -- Gas schedule summary

| Opcode | Name            | Gas formula          | Notes                           |
|--------|-----------------|----------------------|---------------------------------|
| 0x01   | hdc_hamming     | 1,500 flat           | Pure compute, XOR + popcount    |
| 0x02   | hdc_bind        | 500 flat             | Pure compute, XOR               |
| 0x03   | hdc_bundle      | 500 + 100 * count    | Linear in vector count          |
| 0x04   | hdc_permute     | 500 flat             | Bit rotation                    |
| 0x05   | storeVector     | SSTORE-based         | Requires stateful provider      |
| 0x06   | searchSimilar   | Varies (read-heavy)  | Requires stateful provider      |

---

## Step 7 -- Stateful opcodes (0x05, 0x06)

`PrecompileFn` is a plain function pointer -- it cannot access contract
storage.  Opcodes 0x05 and 0x06 need to read/write EVM state.

The solution is to handle them inside `HdcPrecompiles::run()`, which
receives `&mut CTX` (the full EVM context with journal + DB access).
Instead of delegating to the `PrecompileFn`, pattern-match on the opcode
byte directly and use `context.journal()` / `context.journal_mut()` to
perform storage operations:

```rust
// Inside HdcPrecompiles::run(), before calling hdc_precompile():
if inputs.bytecode_address == HDC_ADDRESS {
    let input_bytes = inputs.input.as_bytes(context);
    if !input_bytes.is_empty() {
        match input_bytes[0] {
            0x05 => return self.run_store_vector(context, &input_bytes[1..], inputs),
            0x06 => return self.run_search_similar(context, &input_bytes[1..], inputs),
            _ => { /* fall through to pure PrecompileFn */ }
        }
    }
    // ... existing hdc_precompile() call for pure opcodes
}
```

Implement `run_store_vector` and `run_search_similar` as methods on
`HdcPrecompiles` that call into the journal.  This is a significant piece of
work and is deferred to a follow-up implementation.

---

## Step 8 -- Testing

### 8a -- Unit tests for pure opcodes

Add to `crates/node/executor/src/precompiles/hdc.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hamming_identical_vectors() {
        let input = vec![0u8; 1 + 2 * HDC_DIM_BYTES]; // opcode 0x00 -> will be overridden
        let mut input = vec![0x01]; // opcode
        input.extend_from_slice(&[0xAA; HDC_DIM_BYTES]); // vector a
        input.extend_from_slice(&[0xAA; HDC_DIM_BYTES]); // vector b (identical)
        let result = hdc_precompile(&input, 10_000, 0).unwrap();
        assert!(result.is_success());
        assert_eq!(result.gas_used, GAS_HAMMING);
        // Identical vectors -> distance 0
        assert_eq!(&result.bytes[..], &[0, 0, 0, 0]);
    }

    #[test]
    fn hamming_opposite_vectors() {
        let mut input = vec![0x01]; // opcode
        input.extend_from_slice(&[0x00; HDC_DIM_BYTES]); // all zeros
        input.extend_from_slice(&[0xFF; HDC_DIM_BYTES]); // all ones
        let result = hdc_precompile(&input, 10_000, 0).unwrap();
        assert!(result.is_success());
        // Distance = 10240 bits = 0x00002800
        let distance = u32::from_be_bytes(result.bytes[..4].try_into().unwrap());
        assert_eq!(distance, 10240);
    }

    #[test]
    fn hamming_out_of_gas() {
        let mut input = vec![0x01];
        input.extend_from_slice(&[0x00; 2 * HDC_DIM_BYTES]);
        let result = hdc_precompile(&input, 100, 0).unwrap(); // only 100 gas
        assert!(result.is_halt());
    }

    #[test]
    fn bind_xor() {
        let mut input = vec![0x02];
        input.extend_from_slice(&[0xFF; HDC_DIM_BYTES]);
        input.extend_from_slice(&[0xAA; HDC_DIM_BYTES]);
        let result = hdc_precompile(&input, 10_000, 0).unwrap();
        assert!(result.is_success());
        assert_eq!(result.gas_used, GAS_BIND);
        assert!(result.bytes.iter().all(|&b| b == 0x55)); // 0xFF ^ 0xAA = 0x55
    }

    #[test]
    fn bundle_majority_vote() {
        let mut input = vec![0x03];
        // 3 vectors: two are all-ones, one is all-zeros -> majority = all-ones
        input.extend_from_slice(&[0xFF; HDC_DIM_BYTES]);
        input.extend_from_slice(&[0xFF; HDC_DIM_BYTES]);
        input.extend_from_slice(&[0x00; HDC_DIM_BYTES]);
        let result = hdc_precompile(&input, 10_000, 0).unwrap();
        assert!(result.is_success());
        assert_eq!(result.gas_used, 500 + 100 * 3);
        assert!(result.bytes.iter().all(|&b| b == 0xFF));
    }

    #[test]
    fn permute_zero_shift() {
        let mut input = vec![0x04];
        let v: Vec<u8> = (0..HDC_DIM_BYTES as u8).cycle().take(HDC_DIM_BYTES).collect();
        input.extend_from_slice(&v);
        input.extend_from_slice(&0u32.to_be_bytes()); // n = 0
        let result = hdc_precompile(&input, 10_000, 0).unwrap();
        assert!(result.is_success());
        assert_eq!(&result.bytes[..], &v[..]);
    }

    #[test]
    fn unknown_opcode_halts() {
        let input = vec![0xFF; 100];
        let result = hdc_precompile(&input, 10_000, 0).unwrap();
        assert!(result.is_halt());
    }

    #[test]
    fn empty_input_halts() {
        let result = hdc_precompile(&[], 10_000, 0).unwrap();
        assert!(result.is_halt());
    }

    #[test]
    fn wrong_input_length_halts() {
        let mut input = vec![0x01]; // hamming expects 2560 payload bytes
        input.extend_from_slice(&[0x00; 100]); // only 100
        let result = hdc_precompile(&input, 10_000, 0).unwrap();
        assert!(result.is_halt());
    }
}
```

### 8b -- Integration test (call precompile through the executor)

In `crates/node/executor/tests/` or as a `#[test]` in `revm.rs`:

```rust
#[test]
fn hdc_precompile_via_executor() {
    // 1. Set up a MockStateDb with a funded account.
    // 2. Create a signed EIP-1559 transaction that CALLs address 0x09
    //    with opcode 0x02 (bind) + 2 x 1280 zero bytes.
    // 3. Execute via RevmExecutor::execute().
    // 4. Assert the receipt is successful and gas_used includes the
    //    500-gas precompile cost.
}
```

### 8c -- End-to-end test from Solidity

Deploy a contract that calls the precompile:

```solidity
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.20;

contract HdcTest {
    address constant HDC = address(0x09);

    function hamming(bytes memory a, bytes memory b)
        external view returns (uint32 distance)
    {
        // Encode: opcode 0x01 || a || b
        bytes memory input = abi.encodePacked(uint8(0x01), a, b);
        (bool ok, bytes memory out) = HDC.staticcall(input);
        require(ok, "hamming failed");
        require(out.length == 4, "bad output length");
        distance = uint32(bytes4(out));
    }

    function bind(bytes memory a, bytes memory b)
        external view returns (bytes memory)
    {
        bytes memory input = abi.encodePacked(uint8(0x02), a, b);
        (bool ok, bytes memory out) = HDC.staticcall(input);
        require(ok, "bind failed");
        return out;
    }
}
```

> Note: precompile output is **raw bytes**, not ABI-encoded.  The Solidity
> `staticcall` returns the raw precompile output in the `out` variable.
> There is no ABI length-prefix wrapper.

---

## Anti-patterns

1. **Never use `f64` (or any floating-point) in precompile code.**
   Precompiles run in the consensus path.  Floating-point results vary
   across architectures and compiler versions.  All arithmetic must be
   integer-only.

2. **Never skip gas metering.**  Every opcode must check `gas_limit`
   before performing work and return `PrecompileHalt::OutOfGas` if the
   caller did not supply enough gas.  Forgetting this allows
   denial-of-service.

3. **Never return variable-length output without a fixed encoding.**
   The Solidity `staticcall` returns raw bytes.  If output length depends
   on input (e.g., `searchSimilar` returns `k * 36` bytes), document the
   exact encoding so callers know how to decode.  Do not add ABI encoding
   inside the precompile -- precompiles return raw bytes, and the Solidity
   caller does `abi.decode` or manual slicing.

4. **Never panic in a precompile.**  A panic in the precompile function
   will crash the node.  Use `PrecompileHalt::Other(...)` for all error
   conditions.  The `unwrap()` on the `try_into()` for `n_bytes` in
   `op_permute` is safe because the length is validated above, but prefer
   explicit error handling in production.

---

## RECONCILIATION: This Doc vs. PR #42

PR #42 supersedes this document. The table below summarizes every divergence.

| Aspect | This Document (original plan) | PR #42 (actual implementation) |
|--------|-------------------------------|-------------------------------|
| **HDC precompile address** | `0x09` (replaces BLAKE2F) | `0xA0C` (custom high address) |
| **Stigmergy precompile** | Not planned | `0xA0D` -- `StigmergyPrecompile` |
| **Dispatch method** | Single opcode byte (0x01..0x06) | 4-byte function selectors |
| **Gas schedule** | Per-opcode: hamming=1,500, bind/permute=500, bundle=500+100*N | Flat 50,000 gas for all operations |
| **Crate location** | `crates/node/executor/src/precompiles/` | `crates/precompiles/` (standalone crate) |
| **State model** | Stateful `PrecompileProvider` with journal access | Event-replay: index rebuilt from chain events at block boundaries |
| **Consensus model** | Direct state writes via `HdcPrecompiles::run()` | Deterministic event replay -- no precompile state writes |
| **REVM integration** | Wrap `EthPrecompiles`, intercept at address | Registered via `kora-precompiles` crate, injected into executor |
| **Vector storage** | Opcode 0x05 `storeVector` writes to journal | Events decoded; index rebuilt from `InsightPublished` logs |
| **Vector search** | Opcode 0x06 `searchSimilar` reads journal state | Index queried via precompile; index is ephemeral (rebuilt each block) |

### Which version to follow

**PR #42 is the canonical implementation.** All new precompile work should target:
- `crates/precompiles/` for precompile definitions
- Address `0xA0C` for HDC operations
- Address `0xA0D` for stigmergy operations
- 4-byte selector dispatch
- Event-replay consensus model

This document's approach (address 0x09, opcode bytes, journal state access)
should NOT be used. The original plan was abandoned because:
1. Replacing BLAKE2F at 0x09 creates compatibility risk for future EVM tooling
2. Raw opcode-byte dispatch is non-standard and hard to debug with EVM tooling
3. Stateful precompile providers require deep REVM coupling; event-replay is cleaner
4. The separate `kora-precompiles` crate allows independent versioning

### Source files (PR #42)

| File | Role |
|------|------|
| `crates/precompiles/src/lib.rs` | Crate root, precompile registration |
| `crates/precompiles/src/hdc.rs` | `HDCPrecompile` at `0xA0C` |
| `crates/precompiles/src/stigmergy.rs` | `StigmergyPrecompile` at `0xA0D` |

### Source files (kora-hdc-chain, related)

| File | Role |
|------|------|
| `crates/hdc/chain/src/precompile.rs` | HDC precompile wrapper in chain crate |
| `crates/hdc/chain/src/event.rs` | Event decoding for replay consensus |
| `crates/hdc/chain/src/index.rs` | In-memory vector index (rebuilt from events) |

---

## Remaining Precompile Work Checklist

- [ ] **Merge PR #42** -- brings `kora-precompiles` crate with `HDCPrecompile(0xA0C)` and `StigmergyPrecompile(0xA0D)` into main
- [ ] **Resolve precompile.rs duplication** -- `crates/hdc/chain/src/precompile.rs` defines HDC precompile logic independently from `crates/precompiles/src/hdc.rs`. After merging PR #42, consolidate so there is one canonical precompile definition.
- [ ] **Implement Merkle proofs** -- PR #41 plan for Merkle-proof-based state verification. Not yet implemented in either location.
- [ ] **Wire precompile into test harness** -- `TestApplication` in the executor tests does not register the `kora-precompiles` precompiles. Integration tests calling `0xA0C` will get "precompile not found" errors. Add precompile registration to test setup.
- [ ] **Gas schedule review** -- PR #42 uses a flat 50k gas. Evaluate whether per-operation gas (this doc's approach) would be more accurate for metering. The flat fee overcharges cheap ops (bind, permute) and undercharges expensive ops (bundle of 100 vectors).
- [ ] **Remove dead code** -- If the 0x09 approach is permanently abandoned, remove any vestiges of the original plan from executor source files (if any were partially merged).

---

## Verification

### Confirming PR #42 precompile registration

```bash
# Check if kora-precompiles crate exists
ls -la crates/precompiles/

# Check HDC precompile address
grep -r "0xA0C\|0x0A0C" crates/precompiles/

# Check stigmergy precompile address
grep -r "0xA0D\|0x0A0D" crates/precompiles/

# Run precompile tests (if available)
cargo test -p kora-precompiles -- --nocapture

# Check chain crate precompile
cargo test -p kora-hdc-chain precompile -- --nocapture
```

5. **Do not mutate `EthPrecompiles.precompiles` directly.**  The
   `precompiles` field is `&'static Precompiles` -- a reference to a
   lazily-initialized global.  Mutating it would affect all EVM instances
   process-wide.  Use the wrapper pattern shown above instead.

6. **Do not forget `warm_addresses()` and `contains()`.**  If a
   precompile address is not in the warm set, the first CALL to it incurs
   a cold-access surcharge (2600 gas).  Standard precompile addresses are
   always warm.  Your custom provider must include `0x09` in both
   `warm_addresses()` and `contains()`.

---

## Checklist

- [ ] `kora-hdc` crate exists at `crates/hdc/` with pure-Rust HDC primitives
      (or stubs for initial compilation).
- [ ] `crates/node/executor/Cargo.toml` -- `kora-hdc` dependency added.
- [ ] `crates/node/executor/src/precompiles/mod.rs` -- created.
- [ ] `crates/node/executor/src/precompiles/hdc.rs` -- created with
      `hdc_precompile` function and `HdcPrecompiles` provider.
- [ ] `crates/node/executor/src/lib.rs` -- `mod precompiles;` added.
- [ ] `crates/node/executor/src/revm.rs` -- `build_mainnet()` replaced
      with `build_evm_with_hdc()` in both `execute()` and `simulate_call()`.
- [ ] `MainBuilder` import removed from `revm.rs` (no longer used).
- [ ] Unit tests pass: `cargo test -p kora-executor`.
- [ ] Opcodes 0x01-0x04 return correct results and consume correct gas.
- [ ] Opcodes 0x05-0x06 return a clean halt (stub) -- not a panic.
- [ ] Out-of-gas conditions return `PrecompileHalt::OutOfGas`, not a panic.
- [ ] Wrong-length inputs return a halt, not a panic.
- [ ] No `f64` anywhere in the precompile code path.
- [ ] Integration test confirms the precompile is callable via a transaction.
- [ ] `estimate_gas` still works (uses `simulate_call`, which also uses the
      custom provider).

---

## Audit Findings

Audit date: 2026-05-08. Audited files:

| File | Role |
|------|------|
| `crates/hdc/chain/src/precompile.rs` | Pure-logic precompile dispatcher + opcode functions |
| `crates/node/executor/src/hdc_precompiles.rs` | REVM `PrecompileProvider` wrapper |
| `crates/node/executor/src/revm.rs` | Executor integration (EVM construction) |
| `crates/node/executor/Cargo.toml` | Dependency wiring |
| `crates/hdc/core/src/vector.rs` | Core HDC algebra (bind, hamming, permute, serialize) |
| `crates/hdc/core/src/bundle.rs` | Core bundle (majority-vote) |
| `crates/hdc/core/src/constants.rs` | Canonical constants (D, BYTES, thresholds) |

### F01 -- Precompile address: MATCH

Spec says `0x09`. Implementation in `crates/hdc/chain/src/precompile.rs:14-17`
defines `PRECOMPILE_ADDRESS` as `Address::new([0x00 * 19, 0x09])`. The wrapper
in `crates/node/executor/src/hdc_precompiles.rs:45` references it via
`kora_hdc_chain::PRECOMPILE_ADDRESS`. Address is consistent everywhere.

### F02 -- Opcode map: DIVERGED from spec

The spec defines opcodes:

| Opcode | Spec name       | Spec I/O                                |
|--------|-----------------|----------------------------------------|
| 0x05   | storeVector     | 32B key + 1280B vec (state write)      |
| 0x06   | searchSimilar   | 1280B query + 4B k -> k x 36B results |

The implementation (`crates/hdc/chain/src/precompile.rs:8-9,48-52`) defines:

| Opcode | Impl name      | Impl I/O                                     |
|--------|----------------|----------------------------------------------|
| 0x05   | vector_id      | 1280B vec -> 32B keccak256 hash              |
| 0x06   | is_similar     | 2 x 1280B vecs -> 32B bool (0 or 1)         |

**This is a semantic break.** Opcodes 0x05 and 0x06 were re-purposed from
stateful store/search operations to stateless pure functions. The spec
acknowledged 0x05/0x06 as stubs requiring stateful `PrecompileProvider`
work. The implementation chose to fill the slots with different, pure
operations instead of leaving them as halting stubs.

Whether this is intentional or accidental needs clarification. If
intentional, the spec must be updated. If the stateful operations are
still planned, `vector_id` and `is_similar` should be assigned new opcode
numbers (e.g., 0x07 and 0x08) to avoid future collision.

### F03 -- Gas costs: ALL DIFFER from spec

| Opcode | Spec gas              | Impl gas (`precompile.rs:20-35`)    | Delta  |
|--------|-----------------------|-------------------------------------|--------|
| 0x01   | 1,500 flat            | 100 flat                            | -93%   |
| 0x02   | 500 flat              | 100 flat                            | -80%   |
| 0x03   | 500 + 100*count       | 100 + 150*count                     | mixed  |
| 0x04   | 500 flat              | 120 flat                            | -76%   |
| 0x05   | SSTORE-based (stub)   | 200 flat (vector_id, not store)     | N/A    |
| 0x06   | varies (stub)         | 110 flat (is_similar, not search)   | N/A    |

Gas costs are significantly lower than spec. For opcodes 0x01-0x04, the
implementation uses the `kora_hdc` core library which operates on `u64`
words (word-level XOR, popcount), which is faster than the byte-level
approach described in the spec. The lower gas may be justified, but the
spec needs updating. The spec's gas schedule was designed around naive
byte-level implementations.

**Risk:** Underpriced gas enables cheaper DoS via bundle with large
`count`. The per-vector cost of 150 gas for 1,280 bytes of processing
may be too low. See Security Concerns below.

### F04 -- Input/output encoding: PARTIALLY DIVERGED

**hamming_distance (0x01):**
- Spec: output is 4 bytes (u32 big-endian).
- Impl (`precompile.rs:116-118`): output is 32 bytes, u32 in bytes 28..32
  (ABI-style left-padded). This is **different** from the spec. The
  implementation pads the u32 to 32 bytes for EVM word alignment, which is
  more Solidity-friendly but breaks the raw-4-byte contract in the spec.

**is_similar (0x06):**
- Impl (`precompile.rs:182-183`): output is 32 bytes with `out[31] = 0|1`.
  Same ABI-style padding. Spec had no `is_similar`, so this is new.

**vector_id (0x05):**
- Impl (`precompile.rs:169-170`): output is 32 bytes (keccak256 hash).
  Spec had `storeVector` here, so this is a different operation entirely.

**bundle (0x03):**
- Spec: count is inferred from `payload.len() / 1280`.
- Impl (`precompile.rs:132-135`): count is explicitly encoded as a 4-byte
  big-endian u32 prefix before the vector data. This is **different**
  from the spec. The explicit count prefix is arguably better (prevents
  ambiguity), but it is a wire-format break.

**bind (0x02), permute (0x04):**
- These match spec encoding semantics. The serialization format uses
  little-endian u64 words (`vector.rs:137-143`), which is consistent
  internally but not explicitly documented in the spec (spec just says
  "1280 bytes").

### F05 -- Architecture: DIVERGED (cleaner than spec)

The spec proposed a `crates/node/executor/src/precompiles/` subdirectory
with `mod.rs` and `hdc.rs`. The implementation uses a simpler flat layout:

- `crates/node/executor/src/hdc_precompiles.rs` -- the `PrecompileProvider`
- `crates/hdc/chain/src/precompile.rs` -- the pure-logic dispatcher

This is connected via `#[path = "hdc_precompiles.rs"] mod hdc_precompiles;`
in `revm.rs:32-34` instead of a separate `mod precompiles;` in `lib.rs`.

This is a reasonable architectural simplification. The inline `#[path]`
attribute is slightly unusual but functional. No separate module in
`lib.rs` was needed.

### F06 -- HDC core library delegation: IMPROVEMENT over spec

The spec inlined all HDC operations (XOR, popcount, bit-rotation) directly
in the precompile. The implementation correctly delegates to `kora_hdc`
core library functions:

- `hamming_distance()` -> `kora_hdc::hamming_distance()` (word-level u64 XOR + popcount)
- `bind()` -> `kora_hdc::bind()` (word-level u64 XOR)
- `bundle()` -> `kora_hdc::bundle()` via `BundleAccumulator`
- `permute()` -> `kora_hdc::permute()` (word-level shift/rotate)
- `vector_id()` -> `kora_hdc::vector_id()` (keccak256)
- `is_similar` -> `kora_hdc::hamming_distance()` + threshold comparison

This is better than the spec because:
1. Core algebra is tested independently in `kora_hdc`.
2. The `u64`-word-level implementations are significantly more efficient
   (160 word ops vs 1,280 byte ops or 10,240 bit ops).
3. `BundleAccumulator` uses `i32` signed counters with +1/-1 semantics,
   which is more robust for tie-breaking than the spec's `u16` counters.

### F07 -- Conditional precompile registration: NOT IN SPEC

`RevmExecutor` in `revm.rs:42-66` has an `hdc_precompile_enabled: bool`
flag with a builder method `.with_hdc_precompile()`. The precompile is
only registered when this flag is true. This is not in the spec, which
assumed the HDC precompile is always present.

This is a reasonable addition for backwards compatibility, but the flag
defaults to `false` (`revm.rs:52`), meaning the HDC precompile is OFF
by default. If a node starts without explicitly enabling it, HDC calls
will fail silently (routed to standard BLAKE2F at 0x09).

### F08 -- `MainBuilder` import: NOT removed

The spec's checklist item says to remove the unused `MainBuilder` import.
In `revm.rs:10`, `MainBuilder` is still imported. Since `ctx.build_mainnet()`
is still used in the `else` branch when HDC is disabled (`revm.rs:281`
and `revm.rs:455`), the import IS still needed. The spec's checklist item
was predicated on unconditional HDC enablement.

---

## Implementation Status

| Spec Item | Status | Notes |
|-----------|--------|-------|
| Address 0x09 | DONE | Correct |
| `PrecompileProvider` wrapper (Approach A) | DONE | `HdcPrecompileProvider` in `hdc_precompiles.rs` |
| `warm_addresses()` includes 0x09 | DONE | Line 83-86 of `hdc_precompiles.rs` |
| `contains()` includes 0x09 | DONE | Line 89-91 of `hdc_precompiles.rs` |
| kora-hdc dependency | DONE | via `kora-hdc-chain` (not `kora-hdc` directly) |
| Opcode 0x01 hamming | DONE | Delegates to `kora_hdc::hamming_distance`, output padded to 32B |
| Opcode 0x02 bind | DONE | Delegates to `kora_hdc::bind` |
| Opcode 0x03 bundle | DONE | Delegates to `kora_hdc::bundle`, uses explicit count prefix |
| Opcode 0x04 permute | DONE | Delegates to `kora_hdc::permute` |
| Opcode 0x05 storeVector | NOT DONE | Replaced with `vector_id` (pure, stateless) |
| Opcode 0x06 searchSimilar | NOT DONE | Replaced with `is_similar` (pure, stateless) |
| Stateful `PrecompileProvider` for 0x05/0x06 | NOT DONE | No journal/DB access in provider |
| `build_mainnet()` replaced | PARTIAL | Conditional: only when `hdc_precompile_enabled` is true |
| `execute()` integration | DONE | `revm.rs:446-456` |
| `simulate_call()` integration | DONE | `revm.rs:273-282` |
| `estimate_gas()` integration | DONE | Inherits from `simulate_call` |
| Unit tests for pure opcodes | DONE | `precompile.rs:200-325`, 8 test functions |
| Integration test via executor | NOT DONE | No test exercising the full `RevmExecutor` -> precompile path |
| Solidity end-to-end test | NOT DONE | No contract-level test |
| No `f64` in consensus path | PASS | `similarity()` exists in `kora_hdc::vector` but is not called from precompile |

---

## Anti-Patterns & Duct Tape

### AP01 -- `unwrap()` in consensus-critical code

**File:** `crates/hdc/chain/src/precompile.rs:158`
```rust
let n_bytes: [u8; 4] = data[BYTES..BYTES + 4].try_into().unwrap();
```

This `unwrap()` is technically safe because `data.len() >= BYTES + 4` is
checked at line 154. However, in consensus-critical code, `unwrap()` is
an anti-pattern because a panic crashes the node. Prefer explicit error
handling:
```rust
let n_bytes: [u8; 4] = data[BYTES..BYTES + 4]
    .try_into()
    .map_err(|_| PrecompileError::InvalidInput("permute: n-bytes slice".into()))?;
```

The spec itself acknowledged this pattern at line 796-798.

### AP02 -- `#[path]` attribute for module inclusion

**File:** `crates/node/executor/src/revm.rs:32-33`
```rust
#[path = "hdc_precompiles.rs"]
mod hdc_precompiles;
```

Using `#[path]` to include a sibling file is unusual Rust. The standard
approach is either:
- A `mod hdc_precompiles;` in `lib.rs` (standard module system), or
- A `precompiles/` subdirectory as the spec proposed.

The `#[path]` trick works but makes module hierarchy harder to follow.
The file `hdc_precompiles.rs` is not visible in `lib.rs`'s module tree.

### AP03 -- Error message leaked in output bytes

**File:** `crates/node/executor/src/hdc_precompiles.rs:68-75`
```rust
Err(e) => {
    let mut gas = Gas::new(gas_limit);
    gas.spend_all();
    Ok(Some(InterpreterResult {
        result: InstructionResult::PrecompileError,
        gas,
        output: Bytes::from(e.to_string().into_bytes()),
    }))
}
```

Error messages from the precompile are placed into the `output` field of
the `InterpreterResult`. This means the error string is returned as
the `returndata` to the calling contract. While not strictly incorrect,
this is unusual for precompiles (standard precompiles return empty bytes
on error). The error string content could change between versions,
creating a fragile API if any contract depends on parsing it.

### AP04 -- Macro duplication in `revm.rs`

**File:** `crates/node/executor/src/revm.rs:252-271` and `413-444`

Two `macro_rules!` macros (`simulate!` and `execute_block!`) exist to
avoid duplicating the EVM call logic between the HDC-enabled and
non-HDC paths. This is duct tape around the conditional precompile
feature. A cleaner approach would be a generic helper function that
accepts any `PrecompileProvider` implementor, or always using
`HdcPrecompileProvider` (which already delegates to `EthPrecompiles`
for non-HDC addresses).

### AP05 -- `warm_addresses()` may emit duplicate 0x09

**File:** `crates/node/executor/src/hdc_precompiles.rs:83-86`
```rust
fn warm_addresses(&self) -> Box<impl Iterator<Item = Address>> {
    let hdc_addr = kora_hdc_chain::PRECOMPILE_ADDRESS;
    let inner_addrs = self.inner.warm_addresses();
    Box::new(inner_addrs.chain(std::iter::once(hdc_addr)))
}
```

Since `0x09` is BLAKE2F in CANCUN, `self.inner.warm_addresses()` already
includes `0x09`. This chains another `0x09` at the end, producing a
duplicate. While REVM likely tolerates duplicates in the warm set, it is
sloppy. The spec's version (`precompile-integration.md:457-462`)
explicitly deduplicated with `if !addrs.contains(&HDC_ADDRESS)`.

### AP06 -- Bundle count validation is insufficient

**File:** `crates/hdc/chain/src/precompile.rs:131-148`

The bundle opcode reads `count` from a 4-byte prefix but does not
validate that `count` is reasonable. A caller could pass `count = 0`
with no vector data, which would result in `gas_cost = 100` (base only)
and the `bundle` function receiving an empty slice. While
`BundleAccumulator` handles this (returns all-zero vector), a zero-count
bundle is semantically meaningless and should be rejected.

Additionally, if `count` does not match `vec_data.len() / BYTES`, the
`read_vector` call at line 143 will return `InvalidInput`, but the error
message will not clearly indicate the count/data mismatch.

### AP07 -- No maximum bound on bundle count

**File:** `crates/hdc/chain/src/precompile.rs:135-136`

`count` is a `u32` (max ~4 billion). While the gas cost scales linearly
and a realistic gas limit caps the actual count, there is no explicit
maximum. A `count` of `u32::MAX` would attempt to allocate
`Vec::with_capacity(4_294_967_295)` at line 141, causing an OOM panic
before gas metering can reject it, because the gas check happens AFTER
the count is parsed but BEFORE the allocation... wait, no: the gas check
is at line 137, before the allocation at line 141. So gas would reject
large counts. However, the multiplication `(count as u64) * 150` could
overflow for very large counts. With `count = u32::MAX`, the gas cost
would be `100 + 4_294_967_295 * 150 = 644_245_094_350`, which fits in
`u64`. So this is safe, but only by coincidence of the gas constant
being small enough.

---

## Security Concerns

### SEC01 -- Gas underpricing for `hamming_distance`

The spec priced hamming at 1,500 gas. The implementation charges 100 gas.
The core operation performs 160 u64 XORs + 160 popcounts. For comparison,
the Ethereum `SHA256` precompile (address 0x02) charges 60 base + 12 per
word. For 1,280 bytes (40 EVM words), SHA256 would cost 540 gas. Hamming
distance at 100 gas is underpriced relative to SHA256 for the same input
size. A DoS attacker could spam hamming calls at 100 gas each with
2,560 bytes of calldata.

**Recommendation:** Price hamming at minimum 500 gas, benchmark against
SHA256 and BLAKE2F on the target hardware.

### SEC02 -- Gas underpricing for `bundle`

Bundle with `count = 100` costs `100 + 100 * 150 = 15,100` gas but
processes `100 * 1,280 = 128,000` bytes of input and performs `100 *
10,240 = 1,024,000` bit-level additions. The per-byte cost is ~0.12 gas,
which is far below EVM's 3-gas-per-byte CALLDATALOAD cost.

**Recommendation:** Benchmark bundle on target hardware and adjust
`BUNDLE_PER_VECTOR` to reflect actual CPU cost relative to other EVM
operations.

### SEC03 -- No calldata cost accounting

The precompile gas costs only cover computation, not the cost of loading
the large input data. A hamming call requires 2,560 bytes of calldata.
At EVM's standard 16 gas per non-zero byte, the calldata alone would
cost ~40,960 gas. The precompile charges only 100 gas for computation.
While calldata gas is charged separately by the EVM before the
precompile runs, the precompile's own gas should reflect its
computational cost relative to the data it processes.

### SEC04 -- `is_similar` threshold is hardcoded and consensus-critical

**File:** `crates/hdc/core/src/constants.rs:16`
```rust
pub const THRESHOLD_HAMMING: u32 = 4_854;
```

**File:** `crates/hdc/chain/src/precompile.rs:180`
```rust
let similar = dist <= THRESHOLD_HAMMING;
```

The similarity threshold is a compile-time constant. If it ever needs
to change, all nodes must upgrade simultaneously or consensus breaks.
Consider making this a chain parameter or passing it as input to the
precompile.

### SEC05 -- BLAKE2F silently replaced

As documented in the spec (lines 18-22), address 0x09 replaces BLAKE2F.
Any contract or tool that assumes BLAKE2F is available at 0x09 will get
incorrect results without any error -- the call will succeed but return
HDC operation output instead of BLAKE2F output. There is no warning or
revert.

This is acknowledged as intentional, but it creates a compatibility hazard
for any Ethereum tooling or contracts deployed to this chain that use
BLAKE2F.

### SEC06 -- Error output leaks internal strings

As noted in AP03, error messages are returned in `output` bytes. An
attacker could use this to fingerprint node software versions or
gather information about internal implementation details. Standard
precompiles return empty bytes on error.

---

## Recommended Changes Checklist

- [ ] **CRITICAL: Resolve opcode 0x05/0x06 semantics.** Decide whether
      `vector_id`/`is_similar` replace or coexist with the spec's
      `storeVector`/`searchSimilar`. If coexisting, assign new opcode
      numbers. Update spec accordingly.
      - Files: `crates/hdc/chain/src/precompile.rs:39-52`

- [ ] **HIGH: Revise gas schedule.** Benchmark all opcodes on target
      hardware and reprice. Current costs are 76-93% below spec values.
      Ensure gas reflects actual CPU cost relative to standard EVM ops.
      - File: `crates/hdc/chain/src/precompile.rs:20-35`

- [ ] **HIGH: Update spec to match bundle encoding.** The implementation
      uses an explicit 4-byte count prefix; the spec infers count from
      input length. Pick one and document it.
      - File: `crates/hdc/chain/src/precompile.rs:132-135`
      - Spec: lines 235-236

- [ ] **HIGH: Update spec to match hamming output encoding.** The
      implementation returns 32 bytes (ABI-padded u32); the spec says
      4 bytes. This affects every Solidity caller.
      - File: `crates/hdc/chain/src/precompile.rs:116-118`
      - Spec: line 199

- [ ] **MEDIUM: Replace `unwrap()` with error propagation** in
      `exec_permute`.
      - File: `crates/hdc/chain/src/precompile.rs:158`

- [ ] **MEDIUM: Remove error string from output bytes** on precompile
      errors. Return `Bytes::new()` like standard precompiles.
      - File: `crates/node/executor/src/hdc_precompiles.rs:74`

- [ ] **MEDIUM: Deduplicate 0x09 in `warm_addresses()`.** Either filter
      it from `inner_addrs` or check before appending.
      - File: `crates/node/executor/src/hdc_precompiles.rs:83-86`

- [ ] **MEDIUM: Validate `count > 0` in bundle.** Reject zero-count
      bundles with an error.
      - File: `crates/hdc/chain/src/precompile.rs:132`

- [ ] **MEDIUM: Validate `count * BYTES == vec_data.len()`** explicitly
      in bundle, instead of relying on `read_vector` to fail on short
      data.
      - File: `crates/hdc/chain/src/precompile.rs:140-143`

- [ ] **LOW: Replace `#[path]` with standard module structure.** Either
      add `mod hdc_precompiles;` to `lib.rs` or use a `precompiles/`
      subdirectory.
      - File: `crates/node/executor/src/revm.rs:32-33`

- [ ] **LOW: Eliminate macros in `revm.rs`.** Refactor `simulate!` and
      `execute_block!` into generic functions, or always use
      `HdcPrecompileProvider` (it already passes through to
      `EthPrecompiles` for non-HDC addresses).
      - File: `crates/node/executor/src/revm.rs:252-271, 413-444`

- [ ] **LOW: Add integration test** that exercises the full path:
      `RevmExecutor` with HDC enabled -> transaction calling 0x09 ->
      verify receipt and output.
      - File: `crates/node/executor/src/revm.rs` (test module)

- [ ] **LOW: Document serialization byte order.** The spec says "1280
      bytes" but does not specify endianness. The implementation uses
      little-endian u64 word order (`vector.rs:137-143`). This must be
      documented as part of the ABI.
      - File: `crates/hdc/core/src/vector.rs:137`
      - Spec: throughout Step 3c

- [ ] **LOW: Consider making `THRESHOLD_HAMMING` a precompile input**
      for `is_similar` (opcode 0x06) instead of a compile-time constant,
      to avoid consensus-breaking upgrades when tuning the threshold.
      - File: `crates/hdc/chain/src/precompile.rs:180`
      - File: `crates/hdc/core/src/constants.rs:16`

---

## Second-Pass Remediation Detail

Second-pass scope: `crates/hdc/chain/src/precompile.rs`,
`crates/node/executor/src/hdc_precompiles.rs`, REVM wiring in
`crates/node/executor/src/revm.rs`, and `contracts/src/HdcPrecompile.sol`.
No code was changed by this pass; this section defines the concrete target
state for the next implementation patch.

External references already verified and treated as constraints:

- [EIP-152](https://eips.ethereum.org/EIPS/eip-152) assigns address `0x09`
  to the BLAKE2 `F` compression precompile.
- [EIP-1352](https://eips.ethereum.org/EIPS/eip-1352) reserves the low
  address range `0x0000000000000000000000000000000000000000` through
  `0x000000000000000000000000000000000000ffff` for precompiles and system
  contracts.
- REVM `revm-precompile` v34 defines `PrecompileResult` as
  `Result<PrecompileOutput, PrecompileError>`; `PrecompileError` is fatal,
  while normal invalid-input and out-of-gas conditions should be represented
  as `PrecompileOutput::halt(...)`.

### R01 -- Address conflict and activation policy

`0x09` is a real conflict, not an unused slot. Under Cancun and later
Ethereum specs it is BLAKE2F. The current provider only replaces BLAKE2F
when `RevmExecutor::with_hdc_precompile()` is used; otherwise calls to
`0x09` still route to stock `EthPrecompiles`.

Concrete fix:

- Decide this at chain configuration / hardfork level, not per executor
  builder call. A node must not be able to accidentally run the same chain
  with HDC disabled.
- If HDC stays at `0x09`, document it as an intentional sovereign-chain
  replacement for BLAKE2F and make the HDC-enabled provider the default for
  every block execution, call simulation, and gas estimation path on that
  chain.
- If BLAKE2F compatibility is required, move HDC to a chain-owned reserved
  precompile address in the EIP-1352 range that does not collide with the
  target REVM/Ethereum spec, then update `PRECOMPILE_ADDRESS`,
  `HDC_PRECOMPILE`, genesis/config docs, and tests in one hardfork.
- In either case, `contains()` and `warm_addresses()` must expose the exact
  configured HDC address so the address is treated as a warm precompile.

### R02 -- REVM adapter design

The current adapter manually constructs `InterpreterResult` and ignores the
REVM `PrecompileOutput` conversion path. This loses `reservoir` handling and
returns internal error strings as output bytes.

Concrete fix:

- Keep `HdcPrecompileProvider` as a wrapper around `EthPrecompiles`; the
  wrapper pattern is still the right design because `EthPrecompiles` owns a
  static table and should not be mutated.
- Add an adapter function in the executor layer that maps the HDC domain
  result to REVM's richer result:
  - success -> `Ok(PrecompileOutput::new(gas_used, Bytes::from(output), inputs.reservoir))`
  - out of gas -> `Ok(PrecompileOutput::halt(PrecompileHalt::OutOfGas, inputs.reservoir))`
  - invalid opcode/input -> `Ok(PrecompileOutput::halt(PrecompileHalt::other(...), inputs.reservoir))`
  - truly unrecoverable executor failure only -> `Err(PrecompileError::Fatal(...))`
- Convert to `InterpreterResult` with
  `revm::handler::precompile_provider::precompile_output_to_interpreter_result`
  instead of hand-rolling `Gas` and `InstructionResult`.
- Preserve the stock REVM behavior for top-level non-OOG halt context:
  if `output.halt_reason()` is non-OOG and `context.journal().depth() == 1`,
  set `context.local_mut().set_precompile_error_context(...)`.
- Deduplicate `0x09` in `warm_addresses()`. In Cancun, inner
  `EthPrecompiles` already includes BLAKE2F at `0x09`, so chaining HDC
  unconditionally emits a duplicate.

### R03 -- Gas schedule

The spec and implementation must publish one consensus gas schedule. The
current constants are too low to leave undocumented, especially for calldata
that touches 1,280-byte vectors and for bundle's per-bit accumulator.

Concrete fix:

- Replace the current constants with a conservative hardfork schedule and
  treat future repricing as a hardfork:

| Opcode | Name | Gas formula |
|--------|------|-------------|
| `0x01` | `hamming_distance` | `1_500` |
| `0x02` | `bind` | `1_000` |
| `0x03` | `bundle` | `1_000 + 1_500 * count` |
| `0x04` | `permute` | `1_000` |
| `0x05` | `vector_id` | `1_000` |
| `0x06` | `is_similar` | `1_500` |

- Use checked arithmetic for every formula:
  `checked_mul` for `count * BUNDLE_PER_VECTOR` and `checked_add` for the
  base cost. Overflow should return invalid input or precompile halt, never
  wrap.
- Check gas before allocating the vector list for bundle.
- Add a benchmark follow-up before mainnet parameters are frozen. The table
  above is a safe floor, not a substitute for benchmarking on target
  hardware.

### R04 -- Exact calldata and output ABI

The current Rust implementation and Solidity wrapper disagree. The Rust
implementation should be the source of truth for this stateless release, and
`contracts/src/HdcPrecompile.sol` must be rewritten to match it.

All vectors are exactly `1_280` bytes, serialized as the `kora_hdc`
`HdcVector` layout: 160 `u64` words, word 0 first, each word little-endian.
All precompile inputs are raw bytes, not Solidity ABI tuples.

| Opcode | Input bytes after opcode | Output bytes |
|--------|--------------------------|--------------|
| `0x01` `hamming_distance` | `a[1280] || b[1280]` | 32-byte ABI word with `uint32` distance in bytes `28..32` |
| `0x02` `bind` | `a[1280] || b[1280]` | raw `result[1280]` |
| `0x03` `bundle` | `count:u32_be || vectors[count][1280]` | raw `result[1280]` |
| `0x04` `permute` | `v[1280] || n:u32_be` | raw `result[1280]` |
| `0x05` `vector_id` | `v[1280]` | raw `bytes32` Keccak-256 hash |
| `0x06` `is_similar` | `a[1280] || b[1280]` | 32-byte ABI boolean word, `out[31]` is `0` or `1` |

Concrete Solidity fixes:

- `hamming()` must call opcode `0x01`, require `ret.length == 32`, and
  decode `uint32` from the returned ABI word.
- `bind()` must call opcode `0x02`.
- `bundle()` must call opcode `0x03` and encode `uint32(vectors.length)`,
  not `uint16`.
- `permute()` must call opcode `0x04` and encode `v` before `n`.
- Add `vectorId()` for opcode `0x05` and `isSimilar()` for opcode `0x06`.
- Remove or quarantine `storeVector`, `searchSimilar`, and `deleteVector`
  until stateful opcodes are explicitly designed.
- Every Rust opcode should reject trailing bytes by checking exact payload
  length, not just minimum length.

### R05 -- Bundle bounds

Bundle is the highest-risk input because the caller controls `count`, which
drives allocation, loop work, and gas.

Concrete fix:

- Add `const MAX_BUNDLE_VECTORS: usize = 256;` for the first release.
  Raising this later should be a hardfork parameter change after benchmarks.
- Reject `count == 0`.
- Reject `count > MAX_BUNDLE_VECTORS` before `Vec::with_capacity(count)`.
- Validate exact length with checked math:
  `data.len() == 4 + count * BYTES`.
- Reject both short and trailing-extra bundle calldata with a specific
  invalid-input halt.
- Keep gas linear in `count` and use checked arithmetic before the gas check.

### R06 -- Panic removal

Consensus precompile code must not panic on malformed calldata.

Concrete fix:

- Replace `data[BYTES..BYTES + 4].try_into().unwrap()` in `exec_permute`
  with `get(...).and_then(|s| s.try_into().ok())` mapped to
  `PrecompileError::InvalidInput`.
- Audit the precompile call path for every `unwrap`, `expect`, `assert!`,
  indexing expression, and allocation whose size is caller-controlled.
- Caller-controlled indexing should use `get()` or prior exact-length
  validation. Caller-controlled allocation should be bounded before
  allocation.
- Core helpers whose panics are protected by fixed-size types may remain, but
  the precompile boundary should convert all malformed calldata into a halt
  before those helpers are called.

### R07 -- Error handling

Current invalid-input errors spend all gas and return the formatted Rust error
string as returndata. That creates a fragile contract-facing API and leaks
implementation details.

Concrete fix:

- Return empty output bytes for all HDC precompile halts, matching REVM's
  `precompile_output_to_interpreter_result` behavior.
- Use `PrecompileHalt::OutOfGas` for insufficient gas.
- Use `PrecompileHalt::other_static(...)` or `PrecompileHalt::other(...)`
  for invalid opcode, invalid length, bundle bound violations, and disabled
  stateful opcodes.
- Do not use REVM `PrecompileError` for normal malformed calldata; in REVM it
  represents fatal errors that abort EVM execution.
- If preserving human-readable diagnostics is useful, put them in top-level
  precompile error context like `EthPrecompiles` does, not in contract
  returndata.

### R08 -- Stateful opcode decision

For this release, the HDC precompile should be declared stateless. The
implemented `0x05 = vector_id` and `0x06 = is_similar` are useful pure
operations, but they conflict with the older Solidity wrapper's stateful
`storeVector` / `searchSimilar` model.

Concrete fix:

- Freeze opcodes `0x01..0x06` as the stateless ABI listed in R04.
- Remove stateful methods from `HdcPrecompile.sol` or move them behind a
  separate experimental library that is not advertised as matching the
  precompile.
- Do not later repurpose `0x05` or `0x06` for stateful operations.
- If on-chain indexing is required in a future hardfork, allocate new opcodes
  outside the frozen stateless range, for example:
  - `0x10 store_vector(bytes32 id, bytes vector)`
  - `0x11 delete_vector(bytes32 id)`
  - `0x12 search_similar(bytes query, uint32 top_k)`
- Future stateful opcodes must be implemented inside `HdcPrecompileProvider`
  where `&mut CTX` is available, not inside a pure function that only receives
  calldata and gas. They must define storage layout, access-list/warmth
  behavior, state gas/refunds, maximum index size, deterministic result order,
  and bounded `top_k`.

### R09 -- Tests required for closure

Concrete test additions:

- Unit tests in `crates/hdc/chain/src/precompile.rs` for every opcode's exact
  input length, exact output length, gas used, out-of-gas behavior, unknown
  opcode, empty input, and trailing-byte rejection.
- Bundle-specific tests for `count == 0`, `count > MAX_BUNDLE_VECTORS`,
  short vector data, trailing vector data, below-required gas, and exactly
  `MAX_BUNDLE_VECTORS`.
- Panic-safety tests using malformed calldata for `permute`, `bundle`, and
  `vector_id`; these should assert a precompile error/halt, not a panic.
- Provider tests for HDC-enabled `contains()` and `warm_addresses()` with no
  duplicate HDC address, HDC-disabled behavior still delegating to stock
  BLAKE2F if that mode remains supported, and empty returndata on HDC errors.
- Executor integration tests for both `simulate_call()` and `execute()` with
  `.with_hdc_precompile()` enabled, exercising a transaction that calls
  `0x09`.
- Solidity/Foundry tests for `HdcPrecompile.sol` proving the wrapper opcodes,
  argument order, count width, and return decoding match R04.
- A regression test that confirms malformed HDC calldata cannot return Rust
  error strings to contracts.
