# 18 - Encoding Specs: HDC Wire Formats & Serialization

> **Status: REFERENCE MATERIAL -- still current but needs PR #42 alignment.**
> The core wire formats (vector layout, InsightAnchor, event signatures) remain
> valid. However, the precompile dispatch format diverges between this document
> (raw 1-byte opcodes) and PR #42 (4-byte function selectors). Section 10
> ("PR #42 Selector Tables") was added 2026-05-08 to document the PR #42
> wire format and provide a reconciliation path.

This document specifies the exact byte-level wire formats, serialization rules, and encoding conventions for every HDC data structure that crosses a boundary (disk, network, EVM, RPC). An implementor with no prior HDC context should be able to produce bit-identical output from this spec alone.

---

## Endianness Conventions

Two endianness regimes are in play. Mixing them is a common source of bugs.

| Context | Endianness | Rationale |
|---------|-----------|-----------|
| Rust structs, local storage, P2P wire | **Little-endian** | Native on x86/ARM; matches `u64::to_le_bytes()` |
| EVM ABI encoding, precompile I/O integers | **Big-endian** | EVM word order; matches Solidity `abi.encode` |
| Raw vector payloads | **Little-endian** | Vectors are Rust-native `[u64; 160]` |

**Rule**: integers that flow into the EVM (counts, distances, opcodes in return position) are big-endian. Everything else is little-endian. Vector *payloads* are always little-endian regardless of context because they are opaque blobs to the EVM -- the precompile interprets them.

---

## 1. HdcVector Wire Format

An `HdcVector` is a 10,240-bit binary hyperdimensional vector stored as 160 consecutive `u64` words.

### Layout

```
Offset    Size (bytes)    Field
------    ------------    -----
0         8               word[0]   (u64, little-endian)
8         8               word[1]   (u64, little-endian)
16        8               word[2]   (u64, little-endian)
...       ...             ...
1272      8               word[159] (u64, little-endian)
```

- **Total size**: 160 x 8 = **1,280 bytes** (exactly).
- **Alignment**: 64-byte cache-line alignment. Allocators must return pointers where `ptr % 64 == 0`. This matters for SIMD (AVX-512 `vmovdqa64` requires 64-byte alignment).
- **Dimensionality**: 160 x 64 = 10,240 bits.

### Vector ID Derivation

```
vector_id: [u8; 32] = keccak256(vector_bytes[0..1280])
```

The input to keccak256 is the raw 1,280 bytes in memory order (little-endian words). No length prefix, no padding. The output is 32 bytes, used as a content-address everywhere (on-chain `vectorHash`, P2P frame ID, local DB key).

### Validation

1. Payload must be exactly 1,280 bytes. Reject shorter or longer.
2. Recompute `keccak256(payload)` and compare against any claimed vector ID.
3. No structural constraints on word values -- all 2^10240 bit patterns are valid vectors.

### Worked Example

A vector where every bit is zero:

```
Bytes:     00 00 00 00  00 00 00 00   (word 0)
           00 00 00 00  00 00 00 00   (word 1)
           ...                         (words 2-159, all zero)
Total:     1280 bytes of 0x00
ID:        keccak256(0x0000...0000)  (1280 zero bytes)
         = 0x<32-byte hash>
```

A vector where word[0] = 1 (only the least-significant bit of the first word is set):

```
Bytes:     01 00 00 00  00 00 00 00   (word 0, u64 value 1, little-endian)
           00 00 00 00  00 00 00 00   (word 1)
           ...
```

Note: `01` is the first byte because u64 value `1` in little-endian is `[0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00]`.

---

## 2. InsightAnchor On-Chain Layout

An `InsightAnchor` is the on-chain representation of a published insight. It occupies **95 bytes** across **3 EVM storage slots** (SSTORE).

### Slot Layout

```
Slot 0 (32 bytes):
  Offset    Size    Field
  ------    ----    -----
  0         32      vectorHash [u8; 32]

Slot 1 (32 bytes):
  Offset    Size    Field
  ------    ----    -----
  0         32      contentHash [u8; 32]

Slot 2 (31 bytes packed, right-aligned in 32-byte slot):
  Offset    Size    Field             Solidity Type
  ------    ----    -----             -------------
  0         20      author            address
  20        8       publishBlock      uint64
  28        1       kind              uint8
  29        1       tier              uint8
  30        1       state             uint8
```

### Solidity Packing Rules (Slot 2)

Solidity packs storage variables from right to left within a 32-byte slot. The in-storage byte layout of Slot 2 is:

```
Byte position in slot (0 = most significant):
  [0]          unused padding (1 byte of 0x00)
  [1..20]      author (20 bytes, address)
  [21..28]     publishBlock (8 bytes, uint64, big-endian)
  [29]         kind (1 byte)
  [30]         tier (1 byte)
  [31]         state (1 byte)
```

Total occupied: 20 + 8 + 1 + 1 + 1 = **31 bytes**. One byte of padding at the top of the slot.

### Size Calculation

```
Slot 0:  32 bytes (vectorHash)
Slot 1:  32 bytes (contentHash)
Slot 2:  31 bytes (packed fields)
---------
Total:   95 bytes of data across 3 × 32-byte slots (96 bytes storage)
```

### Field Value Ranges

| Field | Type | Range | Notes |
|-------|------|-------|-------|
| vectorHash | bytes32 | any 32 bytes | keccak256 of the 1,280-byte vector |
| contentHash | bytes32 | any 32 bytes | keccak256 of off-chain content blob |
| author | address | 20 bytes | Ethereum address, no checksum in storage |
| publishBlock | uint64 | 0 to 2^64-1 | Block number at publish time |
| kind | uint8 | 0-5 | Maps to `KnowledgeKind` enum |
| tier | uint8 | 0-3 | Maps to `KnowledgeTier` enum |
| state | uint8 | 0-255 | Lifecycle state (active, archived, etc.) |

### Validation

1. `vectorHash` must be non-zero (zero means uninitialized slot).
2. `kind` must be in range 0..=5.
3. `tier` must be in range 0..=3.
4. `author` must be non-zero address.
5. `publishBlock` must be <= current block number.

---

## 3. InsightEvent ABI Encoding

The `InsightPublished` event is emitted when a new insight is anchored on-chain.

### Solidity Signature

```solidity
event InsightPublished(
    uint256 indexed id,
    address indexed author,
    bytes32 vectorHash,
    uint8   kind
);
```

### Log Structure

EVM logs consist of up to 4 topics (32 bytes each) and a data field (variable length).

```
Topic 0 (32 bytes):  Event selector
    keccak256("InsightPublished(uint256,address,bytes32,uint8)")
    = 0x<32-byte selector hash>

Topic 1 (32 bytes):  insightId
    uint256, left-padded to 32 bytes
    Example: id=42 → 0x000000000000000000000000000000000000000000000000000000000000002a

Topic 2 (32 bytes):  author
    address, left-padded to 32 bytes (12 zero bytes + 20 address bytes)
    Example: 0x000000000000000000000000abcdef0123456789abcdef0123456789abcdef01

Data (64 bytes):
    Offset    Size    Field
    ------    ----    -----
    0         32      vectorHash (bytes32, raw)
    32        32      kind (uint8, left-padded to 32 bytes)
                      Example: kind=2 → 0x0000...0002
```

### Size Calculation

```
Topics:  3 × 32 = 96 bytes  (topic 0 + 2 indexed params)
Data:    2 × 32 = 64 bytes  (vectorHash + kind)
Total:   160 bytes in log entry
```

### Parsing with ethers.js

```javascript
const iface = new ethers.Interface([
  "event InsightPublished(uint256 indexed id, address indexed author, bytes32 vectorHash, uint8 kind)"
]);

// Parse from raw log
const parsed = iface.parseLog({ topics: log.topics, data: log.data });
// parsed.args.id        → BigInt
// parsed.args.author    → string (checksummed address)
// parsed.args.vectorHash → string (0x-prefixed hex, 66 chars)
// parsed.args.kind      → number (0-5)
```

### Compatibility Notes

- `indexed` parameters appear in topics, not data. This is standard Solidity ABI.
- Topic 0 is always the event selector (unless the event is `anonymous`).
- ethers.js v6, viem, and web3.js all handle this encoding natively.
- ABI encoding uses **big-endian** padding. A `uint8` value of `3` in data becomes 31 zero bytes followed by `0x03`.

---

## 4. Precompile Input/Output Encoding

The HDC precompile lives at a fixed address and dispatches on a 1-byte opcode prefix. All vector payloads are raw 1,280-byte little-endian blobs. All integer parameters/returns in the precompile use **big-endian** (EVM convention for `STATICCALL` return data).

### Opcode 0x01: `hdc_hamming`

Computes Hamming distance between two vectors.

```
INPUT (2561 bytes):
  Offset    Size      Field
  ------    ----      -----
  0         1         Opcode: 0x01
  1         1280      vector_a (160 × u64, little-endian)
  1281      1280      vector_b (160 × u64, little-endian)

OUTPUT (4 bytes):
  Offset    Size      Field
  ------    ----      -----
  0         4         distance (u32, big-endian)
```

**Size check**: input must be exactly 2,561 bytes. Output is exactly 4 bytes.

**Distance range**: 0 to 10,240 (total number of bits). Identical vectors yield 0. Maximally different vectors yield 10,240. Fits in u32 (max 4,294,967,295).

**Worked example**: Two identical vectors produce `distance = 0` → output `00 00 00 00`. Two vectors differing in exactly 1 bit produce `distance = 1` → output `00 00 00 01`.

### Opcode 0x02: `hdc_bind`

Computes the binding (XOR) of two vectors.

```
INPUT (2561 bytes):
  Offset    Size      Field
  ------    ----      -----
  0         1         Opcode: 0x02
  1         1280      vector_a (160 × u64, little-endian)
  1281      1280      vector_b (160 × u64, little-endian)

OUTPUT (1280 bytes):
  Offset    Size      Field
  ------    ----      -----
  0         1280      result (160 × u64, little-endian)
```

**Size check**: input must be exactly 2,561 bytes. Output is exactly 1,280 bytes.

**Property**: `bind(a, b) = a XOR b`, word by word. Self-inverse: `bind(bind(a, b), b) = a`.

### Opcode 0x03: `hdc_bundle`

Computes the majority-vote bundle of N vectors.

```
INPUT (5 + count × 1280 bytes):
  Offset            Size            Field
  ------            ----            -----
  0                 1               Opcode: 0x03
  1                 4               count (u32, big-endian)
  5                 count × 1280    vectors (concatenated, each 1280 bytes)

OUTPUT (1280 bytes):
  Offset    Size      Field
  ------    ----      -----
  0         1280      result (160 × u64, little-endian)
```

**Size check**: input must be exactly `5 + count * 1280` bytes. Reject if `count == 0`. Output is exactly 1,280 bytes.

**Tie-breaking**: when count is even and a bit position has exactly count/2 ones, the result bit is 0 (deterministic tie-break to zero).

**Worked example**: bundling 3 vectors with `count = 3` → input is `5 + 3 * 1280 = 3845` bytes. Count field: `00 00 00 03` (big-endian).

### Opcode 0x04: `hdc_permute`

Applies a cyclic bit-permutation N times.

```
INPUT (1285 bytes):
  Offset    Size      Field
  ------    ----      -----
  0         1         Opcode: 0x04
  1         1280      vector (160 × u64, little-endian)
  1281      4         n (u32, big-endian) — number of permutation steps

OUTPUT (1280 bytes):
  Offset    Size      Field
  ------    ----      -----
  0         1280      result (160 × u64, little-endian)
```

**Size check**: input must be exactly 1,285 bytes. Output is exactly 1,280 bytes.

**Permutation semantics**: each step is a circular left-shift by 1 bit across the entire 10,240-bit vector. `n` steps = circular left-shift by `n` bits. `n = 0` returns the input unchanged. `n = 10240` also returns the input unchanged (full cycle).

### Summary Table

| Opcode | Name | Input Size | Output Size |
|--------|------|-----------|------------|
| 0x01 | hdc_hamming | 2,561 bytes | 4 bytes |
| 0x02 | hdc_bind | 2,561 bytes | 1,280 bytes |
| 0x03 | hdc_bundle | 5 + N*1,280 bytes | 1,280 bytes |
| 0x04 | hdc_permute | 1,285 bytes | 1,280 bytes |

### Validation (all opcodes)

1. First byte must be a known opcode (0x01-0x04). Unknown opcodes return empty output and consume all gas.
2. Input length must match the exact expected size for the opcode. Wrong length is an error.
3. Vector payloads are not validated structurally -- all bit patterns are legal.
4. The `count` field in `hdc_bundle` must be >= 1.

---

## 5. Knowledge Entry Serialization (Local Persistence)

For local on-disk storage (not on-chain, not P2P), knowledge entries are serialized with `bincode` (default) or `rkyv` (zero-copy, optional).

### KnowledgeKind Enum (u8)

```
0 = Fact
1 = Rule
2 = Relation
3 = Observation
4 = Inference
5 = Meta
```

### KnowledgeTier Enum (u8)

```
0 = Ephemeral
1 = Session
2 = Persistent
3 = Anchored
```

### PadState

```
Offset    Size    Field
------    ----    -----
0         8       x (f64, IEEE 754 double, little-endian)
8         8       y (f64, IEEE 754 double, little-endian)
16        8       z (f64, IEEE 754 double, little-endian)
```

Total: **24 bytes**.

f64 byte order: little-endian (Rust `f64::to_le_bytes()`). Example: `1.0_f64` → `00 00 00 00 00 00 F0 3F`.

### Full Knowledge Entry Layout (bincode)

bincode uses variable-length integer encoding for lengths and fixed-size for primitives. The approximate layout:

```
Field             Size (bytes)     Notes
-----             ------------     -----
vector            1,280            Raw HdcVector (160 × u64 LE)
vector_id         32               [u8; 32], keccak256 of vector
kind              1                u8 (KnowledgeKind)
tier              1                u8 (KnowledgeTier)
pad_state         24               3 × f64 LE
created_at        8                u64 timestamp (unix seconds, LE)
updated_at        8                u64 timestamp (unix seconds, LE)
label_len         8                u64 (bincode length prefix, LE)
label_data        variable         UTF-8 bytes
tags_count        8                u64 (bincode vec length prefix, LE)
tags_data         variable         Each tag: 8-byte len + UTF-8 bytes
content_hash      32               [u8; 32]
```

**Approximate total**: 1,280 + 32 + 1 + 1 + 24 + 8 + 8 + ~100 (metadata) = **~1,480 bytes** for a typical entry with a short label and a few tags.

### Validation

1. Deserialize with bincode in strict mode (reject trailing bytes).
2. `kind` must be in 0..=5.
3. `tier` must be in 0..=3.
4. `label_data` must be valid UTF-8.
5. `vector_id` must equal `keccak256(vector)`.
6. `created_at <= updated_at`.

---

## 6. RPC Type Encodings

JSON-RPC endpoints encode HDC types as follows. All JSON responses use standard JSON types.

### Type Mapping

| Rust Type | JSON Encoding | Example |
|-----------|--------------|---------|
| `HdcVector` (1280 bytes) | Hex string, `0x` prefix | `"0xaabbccdd..."` (2560 hex chars + prefix = 2562 chars) |
| `[u8; 32]` (vector ID, hash) | Hex string, `0x` prefix | `"0x1a2b3c..."` (64 hex chars + prefix = 66 chars) |
| `u32` (distance) | JSON number | `4217` |
| `f64` (similarity score) | JSON number | `0.8762` |
| `u64` (block number, timestamp) | JSON string (decimal) | `"18432617"` |
| `address` (20 bytes) | Hex string, `0x` prefix, checksummed | `"0xAbC..."` (42 chars) |
| `KnowledgeKind` | JSON number (u8) | `2` |
| `KnowledgeTier` | JSON number (u8) | `1` |
| `bool` | JSON boolean | `true` |

### Hex Encoding Rules

- Always lowercase hex digits for vector and hash payloads (`0xaabb`, not `0xAABB`).
- Always EIP-55 checksummed for addresses (`0xAbCdEf...`).
- Always include `0x` prefix. Reject bare hex.
- No whitespace or separators within hex strings.

### Example RPC Response

```json
{
  "jsonrpc": "2.0",
  "id": 1,
  "result": {
    "vectorId": "0x4f2c8a...64 hex chars total",
    "vector": "0xfe3201...2560 hex chars total",
    "distance": 1847,
    "similarity": 0.8196,
    "kind": 2,
    "tier": 1,
    "blockNumber": "18432617"
  }
}
```

### Validation

1. Hex strings must have the `0x` prefix, even-length hex body, and correct byte count for the type.
2. `u64` values are serialized as decimal strings (not hex) to avoid JavaScript integer precision loss.
3. `f64` values must be finite (no NaN, no Infinity).
4. Distance values must be in range 0..=10,240.

---

## 7. Framed Wire Format (P2P Sync)

When syncing knowledge entries over the network (P2P gossip, replication), entries are wrapped in a length-prefixed frame.

### Frame Layout

```
Offset    Size (bytes)    Field                   Encoding
------    ------------    -----                   --------
0         4               frame_length            u32, little-endian (total bytes after this field)
4         1280            vector                  HdcVector payload (160 × u64, LE)
1284      32              vector_id               [u8; 32] (keccak256 of bytes 4..1284)
1316      1               kind                    u8 (KnowledgeKind: 0-5)
1317      1               tier                    u8 (KnowledgeTier: 0-3)
1318      8               timestamp               u64, little-endian (unix millis)
```

### Size Calculation

```
Payload:       1280 + 32 + 1 + 1 + 8 = 1,322 bytes
frame_length:  1,322
Total on wire: 4 + 1,322 = 1,326 bytes per frame
```

The `frame_length` field contains the value `1322` encoded as a little-endian u32: `2A 05 00 00`.

### Byte-Level Diagram

```
 0                   1                   2                   3
 0 1 2 3 4 5 6 7 8 9 0 1 2 3 4 5 6 7 8 9 0 1 2 3 4 5 6 7 8 9 0 1
+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
|                     frame_length (u32 LE)                     |
+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
|                                                               |
|                    HdcVector (1280 bytes)                      |
|                  (160 × u64, little-endian)                    |
|                                                               |
+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
|                                                               |
|                   vector_id (32 bytes)                         |
|                 keccak256 of vector bytes                      |
|                                                               |
+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
|    kind (u8)  |    tier (u8)  |                               |
+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+                               |
|                   timestamp (u64 LE, 8 bytes)                 |
|                               +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
|                               |
+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
```

### Framing Protocol

1. Reader consumes exactly 4 bytes for `frame_length`.
2. Reader then consumes exactly `frame_length` bytes for the payload.
3. If the stream ends mid-frame, the frame is incomplete and must be discarded.
4. Frames are self-contained: no inter-frame dependencies, no backreferences.

### Validation

1. `frame_length` must equal exactly 1,322 for this version. Reject other values (future versions may extend the format, but version negotiation must happen out-of-band first).
2. Recompute `keccak256(vector_bytes)` and compare to `vector_id`. Reject on mismatch.
3. `kind` must be 0..=5.
4. `tier` must be 0..=3.
5. `timestamp` should be a reasonable unix-millis value (not zero, not in the far future).

---

## Anti-Patterns

These are mistakes that have caused real bugs. Do not do them.

### 1. Mixing Endianness

**Wrong**: Storing a `u32` distance as little-endian in precompile output.
```
// BAD: precompile returns LE but EVM expects BE
let output = distance.to_le_bytes();
```

**Right**: Precompile integer outputs are big-endian.
```
// GOOD: EVM convention
let output = distance.to_be_bytes();
```

**Wrong**: Converting vector words to big-endian before hashing.
```
// BAD: changes the hash
let bytes: Vec<u8> = words.iter().flat_map(|w| w.to_be_bytes()).collect();
let id = keccak256(&bytes);
```

**Right**: Vector bytes are always little-endian for hashing.
```
// GOOD: consistent with in-memory representation
let bytes: Vec<u8> = words.iter().flat_map(|w| w.to_le_bytes()).collect();
let id = keccak256(&bytes);
```

### 2. Forgetting Alignment

**Wrong**: Allocating vector storage without alignment.
```
// BAD: may segfault on AVX-512 aligned load
let v: Vec<u8> = vec![0u8; 1280];
```

**Right**: Use aligned allocation.
```
// GOOD: 64-byte aligned for SIMD
#[repr(C, align(64))]
struct HdcVector([u64; 160]);
```

### 3. Variable-Length Encoding for Consensus Data

**Wrong**: Using bincode (variable-length integers) for data that goes on-chain or into precompile calls.
```
// BAD: bincode may encode count differently on different versions
let encoded = bincode::serialize(&count)?;
```

**Right**: Fixed-size big-endian for EVM-facing data.
```
// GOOD: deterministic, matches EVM
let encoded = count.to_be_bytes(); // always exactly 4 bytes
```

### 4. Omitting Length Checks

**Wrong**: Assuming precompile input is well-formed.
```
// BAD: panics on short input
let opcode = input[0];
let vector_a = &input[1..1281];
```

**Right**: Validate length before parsing.
```
// GOOD: explicit check
if input.len() != 2561 {
    return Err(PrecompileError::InvalidInputLength);
}
```

### 5. JavaScript Number Precision for u64

**Wrong**: Returning block numbers as JSON numbers.
```json
{ "blockNumber": 18432617123456789 }
```
JavaScript `Number.MAX_SAFE_INTEGER` is 2^53 - 1 = 9,007,199,254,740,991. Values above this lose precision.

**Right**: Return u64 values as decimal strings.
```json
{ "blockNumber": "18432617123456789" }
```

---

## Compatibility with EVM Tooling

### ethers.js v6

```javascript
// Encoding a precompile call (hdc_hamming)
const input = ethers.concat([
  new Uint8Array([0x01]),           // opcode
  vectorABytes,                      // 1280 bytes
  vectorBBytes,                      // 1280 bytes
]);
const result = await provider.call({
  to: HDC_PRECOMPILE_ADDRESS,
  data: ethers.hexlify(input),
});
// result is 0x-prefixed hex of 4 bytes (big-endian u32)
const distance = parseInt(result, 16);
```

### viem

```javascript
import { encodePacked, hexToNumber } from 'viem';

const input = encodePacked(
  ['bytes1', 'bytes', 'bytes'],
  ['0x01', vectorAHex, vectorBHex]
);
const result = await client.call({
  to: HDC_PRECOMPILE_ADDRESS,
  data: input,
});
const distance = hexToNumber(result.data);
```

### ABI Encoding for InsightAnchor Reads

Reading from contract storage uses standard Solidity ABI:

```javascript
const abi = [
  "function getInsight(uint256 id) view returns (bytes32 vectorHash, bytes32 contentHash, address author, uint64 publishBlock, uint8 kind, uint8 tier, uint8 state)"
];
const contract = new ethers.Contract(address, abi, provider);
const insight = await contract.getInsight(42);
// insight.vectorHash → bytes32 hex string
// insight.kind       → number (0-5)
```

### Foundry / cast

```bash
# Call hdc_hamming precompile
cast call $PRECOMPILE_ADDR $(cast concat-hex 0x01 $VECTOR_A_HEX $VECTOR_B_HEX)

# Read InsightAnchor
cast call $CONTRACT "getInsight(uint256)(bytes32,bytes32,address,uint64,uint8,uint8,uint8)" 42
```

---

## Quick Reference: All Sizes

| Structure | Size (bytes) | Notes |
|-----------|-------------|-------|
| HdcVector | 1,280 | 160 x u64 LE |
| Vector ID | 32 | keccak256 of vector |
| InsightAnchor (on-chain) | 95 (3 slots) | 32 + 32 + 31 packed |
| InsightPublished log | 160 | 96 topics + 64 data |
| hdc_hamming input | 2,561 | 1 + 1280 + 1280 |
| hdc_hamming output | 4 | u32 BE |
| hdc_bind input | 2,561 | 1 + 1280 + 1280 |
| hdc_bind output | 1,280 | vector |
| hdc_bundle input | 5 + N*1,280 | 1 + 4 + N*1280 |
| hdc_bundle output | 1,280 | vector |
| hdc_permute input | 1,285 | 1 + 1280 + 4 |
| hdc_permute output | 1,280 | vector |
| Knowledge entry (local) | ~1,480 | vector + metadata |
| P2P frame | 1,326 | 4 + 1322 payload |
| PadState | 24 | 3 x f64 LE |

---

## Audit Findings

Audit performed 2026-05-08 against the `hdc` branch. All file paths are relative to the repository root `/Users/will/dev/nunchi/daeji`.

### F01: Precompile hamming output is 32 bytes, spec says 4 bytes

**Spec** (Section 4, Opcode 0x01): `OUTPUT (4 bytes): distance (u32, big-endian)`

**Implementation** (`crates/hdc/chain/src/precompile.rs`, lines 116-118):
```rust
let mut out = vec![0u8; 32];
out[28..32].copy_from_slice(&dist.to_be_bytes());
Ok((gas::HAMMING_DISTANCE, out))
```

The precompile returns a **32-byte** EVM word (u32 value right-padded in a 32-byte slot) rather than the **4-byte** output the spec declares. The endianness is correct (big-endian, placed at bytes 28..32, matching ABI uint256 left-padding), but this is standard Solidity ABI convention, not the raw 4-byte format the spec describes.

**Verdict**: The implementation follows EVM ABI convention (32-byte word) which is the correct practical choice for precompile interop. The **spec is inaccurate** -- it should document 32-byte ABI-padded output for hamming, or explicitly state the 4-byte vs. 32-byte distinction. The precompile test at line 222 confirms the 32-byte expectation: `u32::from_be_bytes(output[28..32].try_into().unwrap())`.

**Severity**: Spec inaccuracy (implementation is correct for EVM; spec should be updated).

### F02: Precompile `is_similar` (0x06) output is 32 bytes, same ABI convention

**Implementation** (`crates/hdc/chain/src/precompile.rs`, lines 181-183):
```rust
let mut out = vec![0u8; 32];
out[31] = u8::from(similar);
Ok((gas::IS_SIMILAR, out))
```

Returns a 32-byte word with bool at byte 31 (ABI-padded uint256 style). This is consistent with F01 but also not documented in the spec. Follows the same ABI convention as `hamming`.

### F03: Precompile opcodes 0x05 and 0x06 exist in implementation but not in spec

**Spec** (Section 4): Documents opcodes 0x01-0x04 only (hamming, bind, bundle, permute).

**Implementation** (`crates/hdc/chain/src/precompile.rs`, lines 39-52):
```rust
enum Opcode {
    HammingDistance = 0x01,
    Bind = 0x02,
    Bundle = 0x03,
    Permute = 0x04,
    VectorId = 0x05,      // NOT IN SPEC
    IsSimilar = 0x06,     // NOT IN SPEC
}
```

Two additional opcodes are implemented but entirely absent from the encoding spec:
- **0x05 `vector_id`**: Input = 1 opcode byte + 1280 vector bytes. Output = 32-byte keccak256 hash.
- **0x06 `is_similar`**: Input = 1 opcode byte + 2560 vector bytes (two vectors). Output = 32-byte ABI bool.

**Severity**: Spec omission. These opcodes should be fully documented with wire formats.

### F04: Precompile bundle does not validate count == 0

**Spec** (Section 4, Opcode 0x03): "Reject if `count == 0`."

**Implementation** (`crates/hdc/chain/src/precompile.rs`, lines 131-148): The `exec_bundle` function checks `data.len() < 4` but never validates `count == 0`. If `count == 0`, the code proceeds to call `bundle(&[])` (empty slice), which invokes `BundleAccumulator::new().to_vector()` returning the zero vector. This does not error out as the spec requires.

**Severity**: Spec violation. A `count == 0` should return `PrecompileError::InvalidInput`.

### F05: Precompile input length validation is lenient, not exact

**Spec** (Section 4): "Input length must match the exact expected size for the opcode."

**Implementation** (`crates/hdc/chain/src/precompile.rs`):
- `exec_hamming_distance` (line 113-114): Uses `read_vector(data, 0)` then `read_vector(data, BYTES)`. The `read_vector` function (line 95) checks `data.len() < offset + BYTES` (a minimum-length check), not `data.len() == expected`. Trailing bytes are silently ignored.
- `exec_bind` (lines 125-126): Same pattern -- trailing bytes accepted.
- `exec_permute` (line 154): Checks `data.len() < BYTES + 4` -- again a minimum, not exact.
- `exec_bundle` (lines 132-143): Checks `data.len() < 4` for the count prefix, then `read_vector` checks minimums per vector. No exact total-length check.

No function performs the exact `input.len() == expected` check the spec requires. Extra trailing bytes after valid data are silently accepted in all four opcodes.

**Severity**: Spec violation. Lenient parsing could mask malformed inputs.

### F06: KnowledgeKind enum names/values diverge from spec

**Spec** (Section 5):
```
0 = Fact, 1 = Rule, 2 = Relation, 3 = Observation, 4 = Inference, 5 = Meta
```

**Implementation** (`crates/hdc/core/src/knowledge/kind.rs`, lines 7-20):
```rust
pub enum KnowledgeKind {
    Insight,           // maps to 0
    Heuristic,         // maps to 1
    CausalLink,        // maps to 2
    Warning,           // maps to 3
    StrategyFragment,  // maps to 4
    AntiKnowledge,     // maps to 5
}
```

The variant names are completely different: `Fact` vs. `Insight`, `Rule` vs. `Heuristic`, `Relation` vs. `CausalLink`, `Observation` vs. `Warning`, `Inference` vs. `StrategyFragment`, `Meta` vs. `AntiKnowledge`. Additionally, the enum has no `#[repr(u8)]` attribute, so discriminant values are not guaranteed by the Rust language to match the u8 values 0-5 in any serialized form (though they happen to be correct in practice for fieldless enums).

**Severity**: High. Either the spec or the implementation must be updated to match. The lack of `#[repr(u8)]` makes any integer-based serialization fragile.

### F07: KnowledgeTier enum names/values diverge from spec

**Spec** (Section 5):
```
0 = Ephemeral, 1 = Session, 2 = Persistent, 3 = Anchored
```

**Implementation** (`crates/hdc/core/src/knowledge/tier.rs`, lines 10-19):
```rust
pub enum KnowledgeTier {
    Transient,     // maps to 0
    Working,       // maps to 1
    Consolidated,  // maps to 2
    Persistent,    // maps to 3
}
```

Same issue: `Ephemeral` vs. `Transient`, `Session` vs. `Working`, `Persistent` (spec) vs. `Consolidated` (impl), `Anchored` vs. `Persistent`. The semantics have shifted significantly (the spec's tier 2 is `Persistent`, the impl's tier 2 is `Consolidated`). Also no `#[repr(u8)]`.

**Severity**: High. Spec-implementation semantic mismatch.

### F08: KnowledgeEntry has no serialization derives

**Spec** (Section 5): Describes a bincode serialization layout for `KnowledgeEntry`.

**Implementation** (`crates/hdc/core/src/knowledge/entry.rs`): The `KnowledgeEntry` struct has no `Serialize`/`Deserialize` derives. The `KnowledgeKind` and `KnowledgeTier` enums also lack serde/bincode derives. There is no actual serialization code anywhere in the knowledge module.

**Severity**: The spec's bincode layout is entirely aspirational -- no implementation exists.

### F09: Event topic hashes are all zeros (placeholder)

**Spec** (Section 3): Describes `InsightPublished(uint256,address,bytes32,uint8)` with a computed keccak256 selector.

**Implementation** (`crates/hdc/chain/src/event.rs`, lines 9-26):
```rust
pub const INSIGHT_PUBLISHED: B256 = B256::ZERO; // TODO: compute actual hash
pub const INSIGHT_ACCEPTED: B256 = B256::ZERO;   // TODO: compute actual hash
pub const INSIGHT_REJECTED: B256 = B256::ZERO;   // TODO: compute actual hash
pub const INSIGHT_CHALLENGED: B256 = B256::ZERO;  // TODO: compute actual hash
pub const PHEROMONE_DEPOSITED: B256 = B256::ZERO;  // TODO: compute actual hash
```

All five event topic selectors are `B256::ZERO`. The `process_log` function (lines 29-53) is also entirely stub code with `// TODO: ABI-decode log data`. Event parsing is non-functional.

Additionally, the event signature in the code comment (`InsightPublished(bytes32,address,uint256)`) differs from the spec (`InsightPublished(uint256,address,bytes32,uint8)`) -- the parameter order and types are different.

**Severity**: Critical. Event decoding is completely non-functional and the signature does not match the spec.

### F10: Spec describes PadState but implementation has no PadState

**Spec** (Section 5): Describes `PadState` as `{ x: f64, y: f64, z: f64 }` occupying 24 bytes.

**Implementation**: No `PadState` struct exists anywhere in `crates/hdc/` or `crates/kora-hdc/`. It is not in `KnowledgeEntry`, which instead has `balance: f64`, `last_accessed: u64`, and `last_decay_tick: u64` as its state fields.

**Severity**: Spec describes a structure that does not exist in the implementation.

### F11: No P2P frame format implementation

**Spec** (Section 7): Describes a detailed framed wire format for P2P sync with a 4-byte length prefix.

**Implementation**: No frame encoding/decoding exists. The `crates/network/transport/` has a `bundle` module but it is unrelated to HDC framing.

**Severity**: Spec describes a format that is not implemented.

---

## Implementation Status

| Spec Section | Status | Notes |
|---|---|---|
| 1. HdcVector Wire Format | **IMPLEMENTED** | Both crates match spec: 160 x u64 LE, keccak256 ID, 1280 bytes. `#[repr(C, align(64))]` correct. |
| 2. InsightAnchor On-Chain Layout | **STUB** | `OnChainHdcIndex` stores vectors in memory. No actual Solidity storage layout or contract interaction. |
| 3. InsightEvent ABI Encoding | **STUB** | All topic hashes are `B256::ZERO`. `process_log` is a no-op. Signature mismatch with spec. |
| 4. Precompile I/O (0x01-0x04) | **MOSTLY IMPLEMENTED** | Endianness correct. Output size is 32-byte ABI-padded (spec says raw). Missing exact length checks and count==0 validation. |
| 4. Precompile I/O (0x05-0x06) | **IMPLEMENTED (undocumented)** | vector_id and is_similar work correctly but are not in the spec. |
| 5. KnowledgeKind / KnowledgeTier | **DIVERGED** | Enum names and semantics differ from spec. No `#[repr(u8)]`. No serialization derives. |
| 5. Knowledge Entry Serialization | **NOT IMPLEMENTED** | No serde/bincode derives. No serialization code. |
| 5. PadState | **NOT IMPLEMENTED** | Struct does not exist. |
| 6. RPC Type Encodings | **PARTIALLY IMPLEMENTED** | JSON-RPC endpoints exist and work. Uses `alloy_primitives::Bytes` (hex-encoded). `u32` distance returned as JSON number (correct). No explicit hex-case or u64-as-string enforcement visible. |
| 7. P2P Frame Format | **NOT IMPLEMENTED** | No framing code exists. |

---

## Second-Pass Remediation Detail

This section is the canonical remediation target after reading the current Rust precompile, event, RPC, serialization code, and the Solidity contracts. Where this conflicts with earlier draft sections, the decisions below take precedence for implementation.

### Verified External References

- Solidity ABI events: for non-anonymous events, `topics[0]` is `keccak(EVENT_NAME+"("+EVENT_ARGS.map(canonical_type_of).join(",")+")")`; indexed args occupy following topics and non-indexed args are ABI-encoded in `data`. Source: <https://docs.solidity.org/en/latest/abi-spec.html#events>.
- Solidity strict ABI encoding: offsets must be minimal, data areas must not overlap, and no gaps are allowed; Solidity encoders produce strict-mode data. Source: <https://docs.solidity.org/en/latest/abi-spec.html#strict-encoding-mode>.
- Alloy Keccak256: `alloy_primitives::utils::Keccak256` exposes `new`, `update`, and `finalize` methods suitable for deriving event topic constants. Source: <https://docs.rs/alloy-primitives/latest/alloy_primitives/utils/struct.Keccak256.html>.

### Canonical HDC Byte Policy

- `HdcVector` is exactly 1,280 bytes: 160 `u64` words serialized with `u64::to_le_bytes()` in word-index order.
- `vector_hash` / `vector_id` is `keccak256(vector_bytes)` over exactly those 1,280 bytes. There is no length prefix, ABI wrapper, or endian conversion before hashing.
- EVM-facing scalar values are 32-byte Solidity ABI words unless this section explicitly says the value is part of a packed precompile request.
- Packed precompile request scalar fields are fixed-width big-endian fields because they are emitted from Solidity with `abi.encodePacked(uint32(...))` and decoded in Rust with `u32::from_be_bytes`.
- RPC `Bytes` and `B256` values are canonical `0x`-prefixed hex via alloy/jsonrpsee serde. RPC callers still provide raw byte strings semantically; the JSON representation is hex.
- P2P frame scalar fields are little-endian because they are not EVM ABI and are consumed by Rust-only transport code.

### Canonical Precompile ABI V1

Canonical precompile address remains `0x0000000000000000000000000000000000000009`. The v1 request format is a 1-byte opcode followed by packed binary arguments. This is not Solidity strict ABI calldata; it is a small precompile-specific wire format. All vector outputs are raw 1,280-byte vectors. All scalar outputs are 32-byte ABI words to keep `abi.decode(ret, (...))` compatibility.

| Opcode | Operation | Input bytes after opcode | Total input | Output | Output size |
|---|---:|---|---:|---|---:|
| `0x01` | `hamming_distance(a,b)` | `a[1280] || b[1280]` | 2,561 | ABI word, `u32` in bytes `28..32` | 32 |
| `0x02` | `bind(a,b)` | `a[1280] || b[1280]` | 2,561 | raw vector | 1,280 |
| `0x03` | `bundle(vectors)` | `count:u32be || vectors[count][1280]` | `5 + count * 1280` | raw vector | 1,280 |
| `0x04` | `permute(v,n)` | `v[1280] || n:u32be` | 1,285 | raw vector | 1,280 |
| `0x05` | `vector_id(v)` | `v[1280]` | 1,281 | `bytes32` hash | 32 |
| `0x06` | `is_similar(a,b)` | `a[1280] || b[1280]` | 2,561 | ABI bool word, byte `31` is `0x00` or `0x01` | 32 |

Canonical scalar output details:

- `hamming_distance`: bytes `0..28` MUST be zero; bytes `28..32` are `distance.to_be_bytes()`. Valid distance range is `0..=10240`.
- `is_similar`: bytes `0..31` MUST be zero except byte `31`, which MUST be `0x00` or `0x01`.
- `vector_id`: the 32 bytes are the raw Keccak digest, not a padded ABI word around a shorter value.

The current Solidity `HdcLib` is not compatible with this table. It currently maps `0x01` to `storeVector`, `0x02` to `searchSimilar`, `0x03` to `deleteVector`, `0x04` to `bundleVectors`, `0x05` to `bindVectors`, `0x06` to `hamming`, and `0x07` to `permute`. The canonical remediation is to migrate `HdcLib` to the v1 pure-op table above. Stateful index calls must not reuse `0x01..0x06`; reserve a separate future range such as `0x20..0x2f` if the chain still needs stateful precompile indexing.

### Precompile Validation Rules

- Reject empty input.
- Reject unknown opcode.
- Every opcode MUST enforce exact total length and MUST reject trailing bytes.
- `bundle` MUST reject `count == 0`.
- `bundle` MUST compute `expected_len = 1 + 4 + count * 1280` with checked arithmetic before allocation or gas multiplication.
- `bundle` SHOULD impose an implementation maximum count or a gas-derived maximum before allocating `Vec<HdcVector>`.
- All vector arguments are structurally valid if and only if they are exactly 1,280 bytes.
- Gas checks MUST happen before CPU-heavy work and before allocating based on untrusted `count`.
- Errors in REVM should return `PrecompileError` / `PrecompileOOG` as today, but successful scalar outputs must remain the 32-byte ABI words above.

### Solidity Wrapper Migration

Update `contracts/src/HdcPrecompile.sol` to the canonical v1 mapping:

- `hamming(a,b)`: opcode `0x01`, payload `abi.encodePacked(uint8(0x01), a, b)`, return `abi.decode(ret, (uint32))`. The existing 32-byte Rust output is valid ABI for `uint32`.
- `bind(a,b)`: opcode `0x02`, payload `abi.encodePacked(uint8(0x02), a, b)`, require `ret.length == 1280`.
- `bundle(vectors)`: opcode `0x03`, payload starts `abi.encodePacked(uint8(0x03), uint32(vectors.length))`; reject empty arrays and arrays longer than `type(uint32).max`.
- `permute(v,n)`: opcode `0x04`, payload order is `abi.encodePacked(uint8(0x04), v, n)`.
- `vectorId(v)`: add opcode `0x05`, return `bytes32`.
- `isSimilar(a,b)`: add opcode `0x06`, return `bool`.
- `storeVector`, `searchSimilar`, and `deleteVector` are not implemented by the Rust precompile. Either remove them from the contract path and rely on finalized events plus node-local indexing, or define separate stateful opcodes in a v2 spec.

`InsightBoard.submit` currently depends on `HdcLib.searchSimilar` and `HdcLib.storeVector`; with the canonical v1 pure precompile, that duplicate-check path is not deployable. The migration must either move duplicate rejection off-chain / into indexer policy, or implement the reserved stateful opcode range before deploying `InsightBoard`.

### Canonical Contract Event Topics

Topic constants MUST be generated from the exact canonical Solidity event signatures below. Do not use names, indexed modifiers, or parameter names in the signature string; use canonical types only.

| Event | Canonical signature | `topic0` |
|---|---|---|
| `InsightPublished` | `InsightPublished(bytes32,bytes32,address,bytes,bytes,uint8,uint8)` | `0x557abaedefcfd3819afda4adf1fd6b502af02eb2d3969c342e6d18049df5648e` |
| `InsightConfirmed` | `InsightConfirmed(bytes32,address,uint64)` | `0x0f36a6596e3776fdc414918df19684bb2cb7e477fccb8670952ad71372eb1798` |
| `InsightChallenged` | `InsightChallenged(bytes32,bytes32,address)` | `0x7c2394a0e90c77b7708a3f8b1ce1f5f3601b08af62200e582817d0197f7aac72` |
| `InsightStateChanged` | `InsightStateChanged(bytes32,uint8,uint8)` | `0x3bda667176b6a9ba4d9ce0f6c06af7f6eff3ae813b01fcd72e199c1f97b45485` |
| `InsightRenewed` | `InsightRenewed(bytes32,address)` | `0xe5925e21b9e9432ba20dee7124009f177c83d1b1790fae155041dcd720a090af` |
| `InsightPurged` | `InsightPurged(bytes32,address)` | `0x9d2578e184d2d0a3456e9f9f7f41fb077c68665f592a6322889b1d4cc3ab76de` |
| `PheromoneDeposited` | `PheromoneDeposited(bytes32,bytes32,address,uint8,uint64,uint64)` | `0x119033e443dd3efeab9d36b6eefa1b4a4ff293aebe3bab71f05ed571547a895d` |
| `PheromoneConfirmed` | `PheromoneConfirmed(bytes32,address,uint64,uint64)` | `0xda2459b6c3c78ea635c65adda134461fe9f31585eb99bc10a228cbc83ac469bb` |
| `PheromonePruned` | `PheromonePruned(bytes32,address)` | `0x621bd37bd1966b8691cd5cf74c512fd54ed261b3b22f3b324ac6d1f764a331fa` |

The Rust `event.rs` constants currently set all topics to `B256::ZERO`; replace them with generated constants using `alloy_primitives::utils::Keccak256::new()`, `update(signature.as_bytes())`, and `finalize()`, or with checked literal constants covered by tests. The existing `INSIGHT_ACCEPTED` and `INSIGHT_REJECTED` symbols have no matching current Solidity events and should be removed or mapped through `InsightStateChanged`.

### Canonical Event ABI Layouts

`InsightPublished(bytes32 indexed insightId, bytes32 indexed vectorHash, address indexed author, bytes vector, bytes content, uint8 kind, uint8 tier)`:

- Topics: 4 topics, 128 bytes total.
- `topic0`: hash above.
- `topic1`: raw `insightId`.
- `topic2`: raw `vectorHash`.
- `topic3`: ABI-encoded address, 12 zero bytes followed by 20 address bytes.
- Data: strict ABI encoding of `(bytes vector, bytes content, uint8 kind, uint8 tier)`.
- Data head is 4 words = 128 bytes. `vector` offset MUST be `0x80`. Because `vector.length == 1280 == 0x500`, `content` offset MUST be `0x5a0`.
- Data size is `1472 + ceil32(content.length)` bytes: `128` head + `32 + 1280` vector tail + `32 + ceil32(content.length)` content tail.
- Validation: decoded vector length exactly 1,280; `keccak256(vector) == vectorHash`; `kind in 0..=5`; `tier in 0..=3`; strict offsets with no gaps.

Other current contract events:

- `InsightConfirmed`: 3 topics, 32 bytes data. `topic1 = insightId`, `topic2 = confirmer`, data is `uint64 totalConfirmations` as one ABI word.
- `InsightChallenged`: 4 topics, 0 bytes data. `topic1 = targetInsightId`, `topic2 = challengingInsightId`, `topic3 = challenger`.
- `InsightStateChanged`: 2 topics, 64 bytes data. Data is `(uint8 oldState, uint8 newState)`, one ABI word each.
- `InsightRenewed`: 3 topics, 0 bytes data. `topic1 = insightId`, `topic2 = renewer`.
- `InsightPurged`: 3 topics, 0 bytes data. `topic1 = insightId`, `topic2 = purger`.
- `PheromoneDeposited`: 4 topics, 96 bytes data. `topic1 = pheromoneId`, `topic2 = locationHash`, `topic3 = depositor`, data is `(uint8 pType, uint64 intensity, uint64 depositBlock)`.
- `PheromoneConfirmed`: 3 topics, 64 bytes data. Data is `(uint64 newConfirmationCount, uint64 newEffectiveHalfLife)`.
- `PheromonePruned`: 3 topics, 0 bytes data.

### RPC Canonicalization

- `hdc_hammingDistance(a,b)`: inputs are JSON hex `Bytes`; each must decode to exactly 1,280 bytes; output is JSON number `u32`, range `0..=10240`.
- `hdc_similarity(a,b)`: same input validation; output must be finite `f64` in `[0.0, 1.0]`; this method is off-chain only and not consensus input.
- `hdc_bind(a,b)`, `hdc_bundle(vectors)`, and `hdc_encode(text)`: output `Bytes` MUST be exactly 1,280 bytes.
- `hdc_bundle(vectors)`: reject an empty vector list, reject any element whose decoded length is not 1,280, and bound the list length before allocating.
- `hdc_search(query, top_k)`: query length exactly 1,280; canonical `top_k` range is `1..=1000`. Current code clamps to 1000; remediation should also reject `top_k == 0`.
- `hdc_vectorId(vector)`: output `B256` equals `keccak256` of the canonical vector bytes.

### P2P Frame V1 Implementation Plan

No HDC P2P frame exists in code today. Implement the first real format as a versioned fixed-size frame instead of the unversioned draft in Section 7.

Frame prefix:

```
Offset  Size  Field
0       4     frame_length, u32 little-endian, bytes after this field
```

`frame_length` for `HdcInsightFrameV1` is exactly `1420`, encoded as `8c 05 00 00`. Total bytes on wire are exactly `1424`.

Payload:

```
Offset  Size  Field
0       1     version = 0x01
1       1     frame_type = 0x01 (InsightVector)
2       2     flags, u16 little-endian, MUST be 0 for v1
4       32    insight_id, bytes32
36      32    vector_hash, bytes32 = keccak256(vector)
68      32    content_hash, bytes32 = keccak256(content bytes) when known, else zero
100     20    author address, raw 20 bytes
120     8     publish_block, u64 little-endian
128     8     timestamp_ms, u64 little-endian
136     1     kind, u8
137     1     tier, u8
138     1     state, u8
139     1     reserved, MUST be 0
140     1280  vector, 160 u64 words little-endian
```

P2P decoder validation:

- Require `frame_length == 1420`; reject shorter, longer, and partial frames.
- Require `version == 1`, `frame_type == 1`, `flags == 0`, and `reserved == 0`.
- Recompute `keccak256(vector)` and require equality with `vector_hash`.
- Require `kind in 0..=5`, `tier in 0..=3`, and `state in 0..=6`.
- Reject zero `insight_id`, zero `author`, and far-future `timestamp_ms` unless the caller explicitly enables historical-replay mode.

Implementation location should be one of:

- `crates/hdc/chain/src/frame.rs` for HDC-owned encode/decode plus unit tests, then wire it into transport later.
- `crates/network/marshal/src/hdc.rs` only if the frame is immediately coupled to commonware broadcast.

Use manual byte encode/decode or `commonware_codec` with fixed-size fields. Do not use JSON, bincode, serde default enum encoding, or platform-native integer encoding for this frame.

### Compatibility Tests Required

- Vector golden tests: all-zero vector, `word[0] = 1`, and all-`0xff` vector serialize to exact 1,280-byte fixtures and hash to stable `B256` values.
- Precompile golden tests: every opcode above checks exact input length, exact output length, and exact output bytes for simple vectors. Add negative tests for trailing bytes, short input, unknown opcode, `bundle count == 0`, and `bundle count` length overflow.
- Solidity wrapper parity tests: Foundry tests for `HdcLib.hamming`, `bind`, `bundle`, `permute`, `vectorId`, and `isSimilar` must generate the same calldata bytes as Rust fixtures.
- Event topic tests: Rust tests derive every table entry above with Alloy `Keccak256::update` / `finalize`; Solidity tests assert emitted `topics[0]` matches the same literal.
- Event strict ABI tests: emit `InsightPublished` with `vector.length == 1280` and known content length; assert `vector` offset is `0x80`, `content` offset is `0x5a0`, total data length is `1472 + ceil32(content.length)`, and there are no gaps.
- Event decode tests: `process_log` rejects wrong topic counts, wrong data length, wrong address padding, invalid enum values, and `vectorHash != keccak256(vector)`.
- RPC tests: jsonrpsee round trips for `Bytes` and `B256`; invalid vector hex length returns `InvalidVectorLength`; `top_k == 0` is rejected; `top_k > 1000` behavior is documented and tested.
- P2P frame tests: one hex fixture for `HdcInsightFrameV1`, round-trip encode/decode, wrong length rejection, endian checks for `publish_block` and `timestamp_ms`, and hash mismatch rejection.

### Migration Notes

- Existing Rust precompile behavior should be preserved for opcodes `0x01..0x06`, but exact-length and `bundle count == 0` validation must be tightened. That is a consensus-visible change if malformed calls have already been accepted, so schedule it with a protocol version gate if a live network has used these opcodes.
- Existing Solidity `HdcLib` is incompatible with the Rust precompile. Contracts compiled against the current library must be redeployed after the opcode migration; there is no safe wrapper-only fix for already-deployed bytecode.
- `InsightPublished` topic0 in the current markdown draft used the wrong event shape. Indexers must key off `0x557aba...5648e` for the current `IInsightBoard` event.
- `event.rs` zero topic constants mean current event syncing is non-functional. Replacing them with the topic table above is additive for real logs unless a previous devnet emitted synthetic zero-topic events.
- The old unversioned Section 7 P2P frame has no implementation. Treat it as superseded by `HdcInsightFrameV1`; no network migration is required unless an out-of-tree worker implemented the draft.
- Local `KnowledgeEntry` bincode layout remains aspirational until the structs derive serde or implement an explicit codec. Do not persist knowledge entries with default serde enum encodings and later treat those bytes as canonical.

---

## Anti-Patterns & Duct Tape

### AP01: Full code duplication between `hdc/core` and `kora-hdc`

Every source file is duplicated verbatim:
- `crates/hdc/core/src/vector.rs` == `crates/kora-hdc/src/vector.rs` (305 lines, byte-identical)
- `crates/hdc/core/src/constants.rs` == `crates/kora-hdc/src/constants.rs` (23 lines, byte-identical)
- `crates/hdc/core/src/encode.rs` == `crates/kora-hdc/src/encode.rs` (252 lines, byte-identical)
- `crates/hdc/core/src/bundle.rs` == `crates/kora-hdc/src/bundle.rs` (127 lines, byte-identical)

The `hdc/core` crate has additional modules (`knowledge`, `trust`, `context`, `cognitive`) that `kora-hdc` does not. Meanwhile, `kora-hdc` is the crate used by `kora-hdc-chain` (the precompile) and the RPC layer. This creates a two-source-of-truth problem: a bug fixed in one crate can be missed in the other.

**Risk**: Divergence over time. One crate silently drifts, producing different vector IDs or different bundle results across subsystems.

### AP02: Ad-hoc ABI encoding in the precompile

The precompile (`crates/hdc/chain/src/precompile.rs`) hand-rolls all ABI encoding:
- Lines 116-117: Manual `vec![0u8; 32]` + `copy_from_slice` for hamming result
- Lines 135: Manual `u32::from_be_bytes` for count parsing
- Lines 158-159: Manual `data[BYTES..BYTES + 4].try_into()` for permute's `n` parameter
- Lines 181-182: Manual bool-to-byte for is_similar

No use of `alloy_sol_types`, `ethabi`, or any ABI library. While this works, it is fragile and easy to get wrong for more complex types. Each new opcode requires hand-rolling the same patterns.

### AP03: BundleAccumulator uses bit-by-bit loop (O(D) per add)

`crates/hdc/core/src/bundle.rs` and `crates/kora-hdc/src/bundle.rs`, lines 23-30:
```rust
pub fn add(&mut self, vector: &HdcVector) {
    for i in 0..D {         // D = 10,240 iterations
        if vector.bit(i) == 1 {
            self.counts[i] += 1;
        } else {
            self.counts[i] -= 1;
        }
    }
}
```

Each `bit(i)` call (in `vector.rs` line 31-34) does a division, modulo, shift, and mask. This is O(D) per vector with high constant factor. A word-level loop (iterate 160 u64 words, extract bits 64 at a time) would be ~64x faster. The `to_vector()` method (lines 47-55) has the same bit-by-bit pattern.

This is called from the precompile's `exec_bundle`, so it runs on-chain per transaction. For bundling N vectors, total work is O(N * 10,240 * ~5 operations per bit) -- roughly 50K operations per vector.

### AP04: `unsafe impl Send/Sync for HdcVector`

`crates/hdc/core/src/vector.rs` and `crates/kora-hdc/src/vector.rs`, lines 23-24:
```rust
unsafe impl Send for HdcVector {}
unsafe impl Sync for HdcVector {}
```

`HdcVector` is `[u64; 160]` which is already `Send + Sync` by Rust's auto-trait rules. The explicit unsafe impls are unnecessary and misleading -- they suggest interior mutability or raw pointers that do not exist. This is duct tape from an earlier iteration.

### AP05: No validation that `count * BYTES` does not overflow in bundle

`crates/hdc/chain/src/precompile.rs`, line 136:
```rust
let gas_cost = gas::BUNDLE_BASE + (count as u64) * gas::BUNDLE_PER_VECTOR;
```

`count` is a `u32` cast to `usize` (line 135) and then to `u64`. On a 32-bit system, `count as usize` could truncate if `count > u32::MAX` (though this is not possible since count is u32). More concerning, `count * BYTES` (line 143 via `i * BYTES`) could overflow on 32-bit targets when `count` is very large. On 64-bit this is fine since `u32::MAX * 1280 < usize::MAX`, but no explicit check exists.

### AP06: Precompile test confirms 32-byte output, contradicting spec

`crates/hdc/chain/src/precompile.rs`, test at line 222:
```rust
let dist = u32::from_be_bytes(output[28..32].try_into().unwrap());
```

This proves the precompile outputs 32 bytes (not 4). The test suite is internally consistent but contradicts the spec's claim of 4-byte output.

---

## Recommended Changes Checklist

### Spec Updates (update the spec to match reality)

- [ ] **S01**: Section 4 (Precompile I/O): Change hamming output from "4 bytes" to "32 bytes (u32 value right-aligned in ABI uint256 word)" -- `crates/hdc/chain/src/precompile.rs` lines 116-118 are correct for EVM
- [ ] **S02**: Section 4: Add opcodes 0x05 (`vector_id`, input=1281 bytes, output=32 bytes) and 0x06 (`is_similar`, input=2561 bytes, output=32-byte ABI bool) -- see `crates/hdc/chain/src/precompile.rs` lines 48-51
- [ ] **S03**: Section 4 summary table: Add entries for opcodes 0x05 and 0x06
- [ ] **S04**: Section 5: Update `KnowledgeKind` enum values to match implementation: `Insight, Heuristic, CausalLink, Warning, StrategyFragment, AntiKnowledge` -- see `crates/hdc/core/src/knowledge/kind.rs` lines 7-20
- [ ] **S05**: Section 5: Update `KnowledgeTier` enum values to match implementation: `Transient, Working, Consolidated, Persistent` -- see `crates/hdc/core/src/knowledge/tier.rs` lines 10-19
- [ ] **S06**: Section 5: Remove or mark PadState as "not implemented" -- no PadState exists in codebase
- [ ] **S07**: Section 5: Mark bincode layout as aspirational / not yet implemented
- [ ] **S08**: Section 3: Fix event signature to match implementation intent (currently `event.rs` comment says `InsightPublished(bytes32,address,uint256)` which also differs from spec)

### Implementation Fixes (fix the code to match spec intent)

- [ ] **I01**: `crates/hdc/chain/src/precompile.rs` `exec_bundle` (line 135): Add `if count == 0 { return Err(PrecompileError::InvalidInput("count must be >= 1")) }` after parsing count
- [ ] **I02**: `crates/hdc/chain/src/precompile.rs`: Add exact length validation to all opcode handlers. For `exec_hamming_distance`: `if data.len() != 2 * BYTES { return Err(...) }`. For `exec_bind`: same. For `exec_permute`: `if data.len() != BYTES + 4 { return Err(...) }`. For `exec_bundle`: `if data.len() != 4 + count * BYTES { return Err(...) }`
- [ ] **I03**: `crates/hdc/core/src/knowledge/kind.rs` line 7: Add `#[repr(u8)]` to `KnowledgeKind`
- [ ] **I04**: `crates/hdc/core/src/knowledge/tier.rs` line 10: Add `#[repr(u8)]` to `KnowledgeTier`
- [ ] **I05**: `crates/hdc/chain/src/event.rs`: Compute actual keccak256 topic hashes instead of `B256::ZERO`; implement ABI decoding in `process_log`
- [ ] **I06**: `crates/hdc/core/src/knowledge/entry.rs`: Add `#[derive(Serialize, Deserialize)]` and corresponding derives to `KnowledgeKind`, `KnowledgeTier`, `KnowledgeSource` for the bincode persistence path

### Code Quality / Deduplication

- [ ] **C01**: Eliminate `kora-hdc` crate entirely. Have `kora-hdc-chain` and other consumers depend on `hdc-core` (`crates/hdc/core`) directly. Currently these 4 files are byte-identical duplicates: `vector.rs` (305 lines), `constants.rs` (23 lines), `encode.rs` (252 lines), `bundle.rs` (127 lines) -- total 707 lines of pure duplication
- [ ] **C02**: `crates/hdc/core/src/vector.rs` and `crates/kora-hdc/src/vector.rs` lines 23-24: Remove `unsafe impl Send for HdcVector {}` and `unsafe impl Sync for HdcVector {}` -- `[u64; 160]` is already Send+Sync automatically
- [ ] **C03**: `crates/hdc/core/src/bundle.rs` lines 23-30: Replace bit-by-bit accumulation with word-level loop extracting bits 64 at a time for ~64x speedup in the precompile hot path. Same for `to_vector()` at lines 47-55. (And `crates/kora-hdc/src/bundle.rs` if C01 is not done first.)
- [ ] **C04**: Consider using `alloy_sol_types` for ABI encoding/decoding in the precompile instead of manual byte manipulation. This would reduce the risk of endianness or padding bugs as more opcodes are added.

---

## 10. PR #42 Selector Tables (4-Byte Function Selectors)

> **Added 2026-05-08.** PR #42 (`kora-precompiles`) replaces the raw 1-byte
> opcode dispatch documented in Section 4 with Solidity-compatible 4-byte
> function selectors. This section documents the actual wire format used by
> PR #42 and provides selector values for both the HDC precompile (0xA0C) and
> the Stigmergy precompile (0xA0D).

### 10.1 Dispatch Model Change

| Aspect | This Doc (Section 4) | PR #42 |
|--------|---------------------|--------|
| Address | `0x09` | `0xA0C` (HDC), `0xA0D` (Stigmergy) |
| Dispatch | 1-byte opcode at `input[0]` | 4-byte selector at `input[0..4]` |
| Gas | Per-opcode (100-200) | Flat 50,000 per call |
| Payload encoding | `opcode \|\| raw_bytes` | `selector \|\| abi.encode(args)` |
| Return encoding | Raw bytes or right-aligned u256 | ABI-encoded return values |

### 10.2 HDC Precompile Selector Table (Address 0xA0C)

Selectors are the first 4 bytes of `keccak256(signature)`.

| Function | Solidity Signature | Selector (hex) | Input After Selector | Output |
|----------|-------------------|-----------------|---------------------|--------|
| projectBytes | `projectBytes(bytes)` | `keccak256("projectBytes(bytes)")[0:4]` | ABI-encoded `(bytes)` where bytes is raw token data | 1,280-byte HdcVector (ABI-encoded as `bytes`) |
| projectTokens | `projectTokens(string)` | `keccak256("projectTokens(string)")[0:4]` | ABI-encoded `(string)` | 1,280-byte HdcVector (ABI-encoded as `bytes`) |
| search | `search(bytes,uint256)` | `keccak256("search(bytes,uint256)")[0:4]` | ABI-encoded `(bytes query_vector, uint256 top_k)` | ABI-encoded array of `(bytes32 id, uint256 distance)` tuples |
| hammingDistance | `hammingDistance(bytes,bytes)` | `keccak256("hammingDistance(bytes,bytes)")[0:4]` | ABI-encoded `(bytes a, bytes b)` each 1,280 bytes | ABI-encoded `(uint256 distance)` |
| bind | `bind(bytes,bytes)` | `keccak256("bind(bytes,bytes)")[0:4]` | ABI-encoded `(bytes a, bytes b)` each 1,280 bytes | 1,280-byte result (ABI-encoded as `bytes`) |
| bundle | `bundle(bytes[])` | `keccak256("bundle(bytes[])")[0:4]` | ABI-encoded `(bytes[] vectors)` | 1,280-byte result (ABI-encoded as `bytes`) |
| permute | `permute(bytes,uint256)` | `keccak256("permute(bytes,uint256)")[0:4]` | ABI-encoded `(bytes vector, uint256 positions)` | 1,280-byte result (ABI-encoded as `bytes`) |

**Computing selectors (reference):**
```python
from eth_abi import encode
from Crypto.Hash import keccak

def selector(sig: str) -> str:
    k = keccak.new(digest_bits=256)
    k.update(sig.encode())
    return "0x" + k.hexdigest()[:8]

# Example outputs (verify against PR #42 Rust code):
# selector("projectBytes(bytes)")       -> 0x________
# selector("projectTokens(string)")     -> 0x________
# selector("search(bytes,uint256)")     -> 0x________
# selector("hammingDistance(bytes,bytes)") -> 0x________
# selector("bind(bytes,bytes)")         -> 0x________
# selector("bundle(bytes[])")           -> 0x________
# selector("permute(bytes,uint256)")    -> 0x________
```

**Note:** The exact selector hex values must be verified against the Rust
`kora-precompiles` crate. The signatures above are derived from the PR #42
design; if the Rust code uses slightly different signatures (e.g.,
`hamming(bytes,bytes)` vs `hammingDistance(bytes,bytes)`), the selectors will
differ.

### 10.3 Stigmergy Precompile Selector Table (Address 0xA0D)

PR #42 moves pheromone/stigmergy logic from Solidity (`PheromoneRegistry.sol`)
into a second Rust precompile at address `0xA0D`.

| Function | Solidity Signature | Selector (hex) | Input After Selector | Output |
|----------|-------------------|-----------------|---------------------|--------|
| deposit | `deposit(bytes,uint8,uint64)` | `keccak256("deposit(bytes,uint8,uint64)")[0:4]` | ABI-encoded `(bytes location, uint8 pType, uint64 intensity)` | ABI-encoded `(bytes32 pheromoneId)` |
| readPheromones | `readPheromones(bytes,uint8,uint8)` | `keccak256("readPheromones(bytes,uint8,uint8)")[0:4]` | ABI-encoded `(bytes queryVector, uint8 pType, uint8 topK)` | ABI-encoded `(bytes32[] ids, uint64[] sinrValues)` |
| currentIntensity | `currentIntensity(bytes32)` | `keccak256("currentIntensity(bytes32)")[0:4]` | ABI-encoded `(bytes32 pheromoneId)` | ABI-encoded `(uint64 intensity)` |
| confirm | `confirm(bytes32)` | `keccak256("confirm(bytes32)")[0:4]` | ABI-encoded `(bytes32 pheromoneId)` | None (success = no revert) |
| cleanup | `cleanup(bytes32)` | `keccak256("cleanup(bytes32)")[0:4]` | ABI-encoded `(bytes32 pheromoneId)` | None |

### 10.4 Payload Wire Format (PR #42 Style)

```
Precompile input layout (PR #42):
==================================

Offset    Size (bytes)    Field
------    ------------    -----
0         4               Function selector (keccak256(sig)[0:4])
4         32              ABI head pointer or first fixed-size arg
36        ...             Remaining ABI-encoded arguments

For vector arguments (bytes type in ABI):
  - ABI head contains offset pointer to the data section
  - Data section: 32-byte length prefix + raw 1,280-byte vector payload
  - Total for one vector arg: 32 (offset) + 32 (length) + 1280 (data) = 1344 bytes

For two-vector operations (e.g., hammingDistance, bind):
  - Selector (4) + offset_a (32) + offset_b (32) + len_a (32) + data_a (1280) + len_b (32) + data_b (1280)
  - Total: 4 + 32 + 32 + 32 + 1280 + 32 + 1280 = 2,692 bytes

Return layout for hammingDistance:
  - 32 bytes: uint256 distance (right-aligned, big-endian)

Return layout for bind/bundle/permute:
  - 32 bytes: offset pointer (0x0000...0020)
  - 32 bytes: length (0x0000...0500 = 1280)
  - 1280 bytes: vector payload
  - Total: 1,344 bytes

Return layout for search:
  - ABI-encoded dynamic array of (bytes32, uint256) tuples
```

### 10.5 Comparison: Raw Opcode vs 4-Byte Selector

| Property | Raw Opcode (Section 4) | 4-Byte Selector (PR #42) |
|----------|----------------------|--------------------------|
| Dispatch overhead | 1 byte | 4 bytes |
| Solidity compatibility | Requires manual `abi.encodePacked(uint8(opcode), ...)` | Standard `abi.encodeWithSelector(...)` or interface calls |
| Tooling support | None (custom format) | Standard -- ethers.js, cast, foundry all understand selectors |
| Collision risk | 256 opcodes max, no collision with EVM | 2^32 selector space, vanishingly small collision chance |
| ABI encoding | Custom per-opcode | Standard Solidity ABI encoding |
| Gas introspection | Requires custom decoder | Standard tools can decode calldata |

**Recommendation:** Adopt PR #42's 4-byte selector approach. It aligns with
standard EVM tooling, makes the precompile callable via standard Solidity
interfaces, and eliminates the custom encoding/decoding layer.

---

## 11. Event Log Encoding

> **Added 2026-05-08.** Documents the on-chain event log encoding for HDC
> contract events. These are the events that `event.rs` must decode during
> finalized block replay.

### 11.1 InsightBoard Events

#### InsightPublished

```
Topic 0: keccak256("InsightPublished(bytes32,bytes32,address,bytes,bytes,uint8,uint8)")
Topic 1: bytes32 insightId     (indexed)
Topic 2: bytes32 vectorHash    (indexed)
Topic 3: address author        (indexed, left-padded to 32 bytes)
Data:     abi.encode(bytes vector, bytes content, uint8 kind, uint8 tier)
```

Wire layout of data section:
```
Offset   Size    Field
------   ----    -----
0        32      offset to vector (dynamic)
32       32      offset to content (dynamic)
64       32      uint8 kind (right-aligned in 32-byte word)
96       32      uint8 tier (right-aligned in 32-byte word)
128      32      vector length (1280)
160      1280    vector payload
1440     padding to 32-byte boundary (0 bytes -- 1280 is 32*40, already aligned)
1440     32      content length
1472     ...     content payload
```

#### InsightConfirmed

```
Topic 0: keccak256("InsightConfirmed(bytes32,address,uint64)")
Topic 1: bytes32 insightId     (indexed)
Topic 2: address confirmer     (indexed)
Data:     abi.encode(uint64 totalConfirmations)
```

#### InsightChallenged

```
Topic 0: keccak256("InsightChallenged(bytes32,bytes32,address)")
Topic 1: bytes32 insightId              (indexed)
Topic 2: bytes32 challengingInsightId   (indexed)
Topic 3: address challenger             (indexed)
Data:     (empty -- all fields are indexed)
```

#### InsightStateChanged

```
Topic 0: keccak256("InsightStateChanged(bytes32,uint8,uint8)")
Topic 1: bytes32 insightId     (indexed)
Data:     abi.encode(uint8 oldState, uint8 newState)
```

#### InsightRenewed

```
Topic 0: keccak256("InsightRenewed(bytes32,address)")
Topic 1: bytes32 insightId     (indexed)
Topic 2: address renewer       (indexed)
Data:     (empty)
```

#### InsightPurged

```
Topic 0: keccak256("InsightPurged(bytes32,address)")
Topic 1: bytes32 insightId     (indexed)
Topic 2: address purger        (indexed)
Data:     (empty)
```

### 11.2 PheromoneRegistry Events

#### PheromoneDeposited

```
Topic 0: keccak256("PheromoneDeposited(bytes32,bytes32,address,uint8,uint64,uint64)")
Topic 1: bytes32 pheromoneId   (indexed)
Topic 2: bytes32 locationHash  (indexed)
Topic 3: address depositor     (indexed)
Data:     abi.encode(uint8 pType, uint64 intensity, uint64 depositBlock)
```

#### PheromoneConfirmed

```
Topic 0: keccak256("PheromoneConfirmed(bytes32,address,uint64,uint64)")
Topic 1: bytes32 pheromoneId   (indexed)
Topic 2: address confirmer     (indexed)
Data:     abi.encode(uint64 newConfirmationCount, uint64 newEffectiveHalfLife)
```

#### PheromonePruned

```
Topic 0: keccak256("PheromonePruned(bytes32,address)")
Topic 1: bytes32 pheromoneId   (indexed)
Topic 2: address pruner        (indexed)
Data:     (empty)
```

### 11.3 Topic Hash Reference

These are the keccak256 hashes that `event.rs` must use for topic matching.
Replace all `B256::ZERO` constants with these computed values.

```rust
use alloy_primitives::B256;

// InsightBoard events
pub const INSIGHT_PUBLISHED: B256 = /* keccak256("InsightPublished(bytes32,bytes32,address,bytes,bytes,uint8,uint8)") */;
pub const INSIGHT_CONFIRMED: B256 = /* keccak256("InsightConfirmed(bytes32,address,uint64)") */;
pub const INSIGHT_CHALLENGED: B256 = /* keccak256("InsightChallenged(bytes32,bytes32,address)") */;
pub const INSIGHT_STATE_CHANGED: B256 = /* keccak256("InsightStateChanged(bytes32,uint8,uint8)") */;
pub const INSIGHT_RENEWED: B256 = /* keccak256("InsightRenewed(bytes32,address)") */;
pub const INSIGHT_PURGED: B256 = /* keccak256("InsightPurged(bytes32,address)") */;

// PheromoneRegistry events
pub const PHEROMONE_DEPOSITED: B256 = /* keccak256("PheromoneDeposited(bytes32,bytes32,address,uint8,uint64,uint64)") */;
pub const PHEROMONE_CONFIRMED: B256 = /* keccak256("PheromoneConfirmed(bytes32,address,uint64,uint64)") */;
pub const PHEROMONE_PRUNED: B256 = /* keccak256("PheromonePruned(bytes32,address)") */;
```

**Implementation note:** Prefer generating these from `alloy_sol_types::sol!`
macro or the Solidity ABI JSON rather than hand-computing keccak256. The
`alloy_sol_types` approach ensures the topic hashes stay synchronized with the
Solidity source.

### 11.4 Rust Event Decoding Pattern

```rust
use alloy_primitives::{Address, B256, U256};
use alloy_sol_types::{sol, SolEvent};

sol! {
    event InsightPublished(
        bytes32 indexed insightId,
        bytes32 indexed vectorHash,
        address indexed author,
        bytes vector,
        bytes content,
        uint8 kind,
        uint8 tier
    );
}

fn decode_insight_published(topics: &[B256], data: &[u8]) -> Result<InsightPublished, DecodeError> {
    InsightPublished::decode_log_data(topics, data, true)
}
```

### 11.5 Encoding Spec Verification Checklist

- [ ] Verify all selector hex values against actual `kora-precompiles` Rust source
- [ ] Verify event topic hashes against compiled Solidity ABI JSON
- [ ] Confirm `process_log()` in `event.rs` handles all 9 event types listed above
- [ ] Confirm `process_log()` receives full `topics[]` array (not just `topic[0]`)
- [ ] Verify precompile return encoding matches what Solidity's `abi.decode()` expects
- [ ] Add roundtrip tests: Solidity emit -> Rust decode -> verify all fields
