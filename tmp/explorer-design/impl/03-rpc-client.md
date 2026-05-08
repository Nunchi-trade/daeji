# RPC Client Layer -- Implementation Guide

The data layer that connects the Kora Explorer to the kora node.
HTTP JSON-RPC polling today. WebSocket subscriptions when available.

---

## File Map

```
apps/explorer/src/data/
  types.ts          TypeScript interfaces for all chain data
  rpc.ts            viem client setup, custom chain definition, custom actions
  connection.ts     ConnectionManager -- reconnect, health check, state machine
  poller.ts         Block poller -- 1s interval, skip-on-same, header-first optimization
  cache.ts          LRUMap<K,V> -- generic, configurable, memory-bounded
  bus.ts            EventBus (mitt-based, typed)
  store.ts          Zustand chain state store
```

---

## 1. Current RPC State (What Exists Today)

### Transport

| Property       | Value                              |
|----------------|------------------------------------|
| Protocol       | HTTP JSON-RPC 2.0                  |
| Port           | 8545                               |
| WebSocket      | **NOT AVAILABLE** (no WS endpoint) |
| Authentication | None                               |
| TLS            | None (plaintext HTTP)              |

### Available Methods

**Standard Ethereum JSON-RPC:**

| Method                        | Parameters                                    | Returns                        |
|-------------------------------|-----------------------------------------------|--------------------------------|
| `eth_blockNumber`             | none                                          | `QUANTITY` -- latest block num |
| `eth_getBlockByNumber`        | `QUANTITY\|TAG`, `Boolean` (full txns)        | `Block` or `null`              |
| `eth_getBlockByHash`          | `DATA` (32 bytes), `Boolean` (full txns)      | `Block` or `null`              |
| `eth_getBalance`              | `DATA` (20 bytes), `QUANTITY\|TAG`            | `QUANTITY` -- balance in wei   |
| `eth_getCode`                 | `DATA` (20 bytes), `QUANTITY\|TAG`            | `DATA` -- bytecode             |
| `eth_getTransactionByHash`    | `DATA` (32 bytes)                             | `Transaction` or `null`        |
| `eth_getTransactionReceipt`   | `DATA` (32 bytes)                             | `Receipt` or `null`            |
| `eth_getLogs`                 | `FilterObject`                                | `Log[]`                        |
| `eth_gasPrice`                | none                                          | `QUANTITY` -- gas price in wei |
| `eth_feeHistory`              | `QUANTITY`, `QUANTITY\|TAG`, `Number[]`        | `FeeHistory`                   |
| `eth_call`                    | `CallObject`, `QUANTITY\|TAG`                 | `DATA` -- return data          |
| `eth_estimateGas`             | `CallObject`                                  | `QUANTITY` -- gas estimate     |
| `eth_sendRawTransaction`      | `DATA` -- signed tx bytes                     | `DATA` -- tx hash              |

**Kora-specific methods:**

| Method              | Parameters | Returns                                                                                                 |
|---------------------|------------|---------------------------------------------------------------------------------------------------------|
| `kora_nodeStatus`   | none       | `{ chainId, validatorIndex, currentView, finalizedCount, proposedCount, nullifiedCount, peerCount, isLeader }` |

**HDC methods (Hyperdimensional Computing):**

| Method             | Parameters                        | Returns                         |
|--------------------|-----------------------------------|---------------------------------|
| `hdc_hammingDistance` | `DATA`, `DATA`                  | `QUANTITY` -- hamming distance  |
| `hdc_similarity`     | `DATA`, `DATA`                  | `Number` -- cosine similarity   |
| `hdc_bind`           | `DATA[]`                        | `DATA` -- bound hypervector     |
| `hdc_bundle`         | `DATA[]`                        | `DATA` -- bundled hypervector   |
| `hdc_search`         | `DATA`, `QUANTITY` (top-k)      | `{ id, similarity }[]`         |
| `hdc_vectorId`       | `DATA`                          | `DATA` -- vector identifier     |
| `hdc_encode`         | `DATA`, `String` (encoding type)| `DATA` -- encoded hypervector   |

### What Does NOT Exist

- No `eth_subscribe` / `eth_unsubscribe` (no WebSocket)
- No `debug_*` methods
- No `trace_*` methods
- No `kora_subscribe` (consensus subscription is planned, not implemented)
- No pending transaction pool access via RPC

---

## 2. TypeScript Type Definitions

Complete type definitions for all chain data structures. These live in a single file
and are imported throughout the data layer.

```typescript
// apps/explorer/src/data/types.ts

// ================================================================
// HEX BRANDED TYPE
// ================================================================

/** Hex-encoded string, always prefixed with 0x */
type Hex = `0x${string}`;

// ================================================================
// BLOCK
// ================================================================

/** Block as returned by eth_getBlockByNumber with full transactions */
export interface ChainBlock {
  number: bigint;
  hash: Hex;
  parentHash: Hex;
  timestamp: bigint;
  gasUsed: bigint;
  gasLimit: bigint;
  baseFeePerGas: bigint;
  transactions: Hex[] | ChainTransaction[];
  stateRoot: Hex;
  transactionsRoot: Hex;
  receiptsRoot: Hex;

  // -- Computed locally, not from RPC --
  _activityLevel: number;   // gasUsed / gasLimit, range 0..1
  _arrivalTime: number;     // Date.now() when the explorer received this block
}

// ================================================================
// TRANSACTION
// ================================================================

export interface ChainTransaction {
  hash: Hex;
  from: Hex;
  to: Hex | null;            // null = contract creation
  value: bigint;
  gas: bigint;
  gasPrice: bigint;
  input: Hex;
  nonce: bigint;
  blockNumber: bigint;
  blockHash: Hex;
  transactionIndex: number;
  type: number;               // 0=legacy, 1=2930, 2=1559
}

// ================================================================
// RECEIPT
// ================================================================

export interface ChainReceipt {
  transactionHash: Hex;
  status: 'success' | 'reverted';
  blockNumber: bigint;
  blockHash: Hex;
  from: Hex;
  to: Hex | null;
  gasUsed: bigint;
  cumulativeGasUsed: bigint;
  contractAddress: Hex | null;
  logs: ChainLog[];
}

// ================================================================
// LOG
// ================================================================

export interface ChainLog {
  address: Hex;
  topics: Hex[];
  data: Hex;
  blockNumber: bigint;
  transactionHash: Hex;
  logIndex: number;
}

// ================================================================
// NODE STATUS (kora-specific)
// ================================================================

export interface NodeStatus {
  chainId: number;
  validatorIndex: number;
  currentView: number;
  finalizedCount: number;
  proposedCount: number;
  nullifiedCount: number;
  peerCount: number;
  isLeader: boolean;
}

// ================================================================
// FEE DATA
// ================================================================

export interface FeeDataPoint {
  blockNumber: bigint;
  baseFee: bigint;
  gasUsedRatio: number;
  reward25: bigint;
  reward50: bigint;
  reward75: bigint;
}

// ================================================================
// ADDRESS METADATA
// ================================================================

export interface AddressMeta {
  address: Hex;
  balance: bigint;
  nonce: number;
  isContract: boolean;
  firstSeen: bigint;          // block number
  lastSeen: bigint;           // block number
  txCount: number;            // local count from cached blocks
}

// ================================================================
// PENDING TRANSACTION (Phase 2 only)
// ================================================================

export interface PendingTransaction {
  hash: Hex;
  from: Hex;
  to: Hex | null;
  value: bigint;
  gasPrice: bigint;
  submittedAt: number;        // Date.now() when seen
  status: 'pending' | 'included' | 'dropped';
}

// ================================================================
// EVENT BUS TYPES
// ================================================================

export type BusEvents = {
  'block:new': ChainBlock;
  'tx:confirmed': ChainTransaction;
  'tx:pending': PendingTransaction;
  'tx:resolved': { hash: Hex; status: 'included' | 'dropped' };
  'fee:update': FeeDataPoint[];
  'address:seen': Hex;
  'log:new': ChainLog;
  'node:status': NodeStatus;
  'connection:status': ConnectionState;
};

// ================================================================
// CONNECTION STATE
// ================================================================

export type ConnectionState =
  | 'connecting'
  | 'connected'
  | 'reconnecting'
  | 'disconnected';
```

### Implementation Checklist -- types.ts

- [ ] File created at `apps/explorer/src/data/types.ts`
- [ ] All interfaces exported with `export` keyword
- [ ] `Hex` type alias defined as `` `0x${string}` ``
- [ ] `ChainBlock.transactions` is union type: `Hex[] | ChainTransaction[]` (supports both full and hash-only responses)
- [ ] `ChainTransaction.to` is `Hex | null` (null for contract creation)
- [ ] `ChainReceipt.status` is string literal union `'success' | 'reverted'` (not boolean)
- [ ] `NodeStatus` includes all 8 fields from `kora_nodeStatus` response
- [ ] `ConnectionState` is a 4-value string literal union (connecting, connected, reconnecting, disconnected)
- [ ] `BusEvents` maps event names to payload types (used with mitt for type-safe events)
- [ ] `ChainBlock._activityLevel` and `_arrivalTime` prefixed with underscore (computed locally, not from RPC)
- [ ] No `any` types anywhere in the file
- [ ] File compiles under `strict: true` TypeScript config

---

## 3. viem Client Setup (rpc.ts)

### Custom Kora Chain Definition

```typescript
// apps/explorer/src/data/rpc.ts

import {
  createPublicClient,
  http,
  type PublicClient,
  type HttpTransport,
  type Chain,
} from 'viem';
import { defineChain } from 'viem/chains';
import type { NodeStatus } from './types';

// ================================================================
// CHAIN DEFINITION
// ================================================================

/**
 * Custom chain definition for the Kora network.
 * Chain ID and RPC URL are read from environment variables
 * with sensible defaults for local/dev usage.
 */
export const kora: Chain = defineChain({
  id: Number(import.meta.env.VITE_CHAIN_ID ?? 1337),
  name: 'Kora',
  nativeCurrency: {
    name: 'Ether',
    symbol: 'ETH',
    decimals: 18,
  },
  rpcUrls: {
    default: {
      http: [import.meta.env.VITE_RPC_HTTP ?? 'http://localhost:8545'],
    },
  },
});

// ================================================================
// HTTP CLIENT (Phase 1: polling + on-demand queries)
// ================================================================

export const client: PublicClient<HttpTransport, typeof kora> = createPublicClient({
  chain: kora,
  transport: http(import.meta.env.VITE_RPC_HTTP ?? 'http://localhost:8545', {
    retryCount: 3,
    retryDelay: 1000,
    timeout: 10_000,
  }),
});

// ================================================================
// CUSTOM ACTIONS: kora_nodeStatus
// ================================================================

/**
 * Fetch node status via kora_nodeStatus RPC method.
 * This is a kora-specific extension, not part of standard Ethereum JSON-RPC.
 */
export async function getNodeStatus(): Promise<NodeStatus> {
  const result = await client.request({
    method: 'kora_nodeStatus' as any,
    params: [] as any,
  });

  const raw = result as Record<string, unknown>;

  return {
    chainId: Number(raw.chainId),
    validatorIndex: Number(raw.validatorIndex),
    currentView: Number(raw.currentView),
    finalizedCount: Number(raw.finalizedCount),
    proposedCount: Number(raw.proposedCount),
    nullifiedCount: Number(raw.nullifiedCount),
    peerCount: Number(raw.peerCount),
    isLeader: Boolean(raw.isLeader),
  };
}

// ================================================================
// CUSTOM ACTIONS: hdc_* methods
// ================================================================

export async function hdcHammingDistance(a: `0x${string}`, b: `0x${string}`): Promise<bigint> {
  const result = await client.request({
    method: 'hdc_hammingDistance' as any,
    params: [a, b] as any,
  });
  return BigInt(result as string);
}

export async function hdcSimilarity(a: `0x${string}`, b: `0x${string}`): Promise<number> {
  const result = await client.request({
    method: 'hdc_similarity' as any,
    params: [a, b] as any,
  });
  return Number(result);
}

export async function hdcBind(vectors: `0x${string}`[]): Promise<`0x${string}`> {
  const result = await client.request({
    method: 'hdc_bind' as any,
    params: [vectors] as any,
  });
  return result as `0x${string}`;
}

export async function hdcBundle(vectors: `0x${string}`[]): Promise<`0x${string}`> {
  const result = await client.request({
    method: 'hdc_bundle' as any,
    params: [vectors] as any,
  });
  return result as `0x${string}`;
}

export async function hdcSearch(
  query: `0x${string}`,
  topK: number,
): Promise<Array<{ id: `0x${string}`; similarity: number }>> {
  const result = await client.request({
    method: 'hdc_search' as any,
    params: [query, `0x${topK.toString(16)}`] as any,
  });
  return (result as Array<{ id: string; similarity: string }>).map((r) => ({
    id: r.id as `0x${string}`,
    similarity: Number(r.similarity),
  }));
}

export async function hdcVectorId(vector: `0x${string}`): Promise<`0x${string}`> {
  const result = await client.request({
    method: 'hdc_vectorId' as any,
    params: [vector] as any,
  });
  return result as `0x${string}`;
}

export async function hdcEncode(
  data: `0x${string}`,
  encodingType: string,
): Promise<`0x${string}`> {
  const result = await client.request({
    method: 'hdc_encode' as any,
    params: [data, encodingType] as any,
  });
  return result as `0x${string}`;
}
```

### Environment Variables

```
# apps/explorer/.env
VITE_RPC_HTTP=http://localhost:8545
VITE_CHAIN_ID=1337
```

```
# apps/explorer/.env.production
VITE_RPC_HTTP=http://65.109.61.210:8545
VITE_CHAIN_ID=1337
```

### Implementation Checklist -- rpc.ts

- [ ] File created at `apps/explorer/src/data/rpc.ts`
- [ ] `kora` chain defined via `defineChain` with id from `VITE_CHAIN_ID` (default 1337)
- [ ] `client` created via `createPublicClient` with `http` transport
- [ ] Transport configured with `retryCount: 3`, `retryDelay: 1000`, `timeout: 10_000`
- [ ] `getNodeStatus()` calls `kora_nodeStatus` and returns typed `NodeStatus` object
- [ ] `getNodeStatus()` converts all numeric fields via `Number()` (RPC returns hex strings)
- [ ] All `hdc_*` functions exported as named async functions
- [ ] `hdcHammingDistance` returns `bigint`
- [ ] `hdcSimilarity` returns `number` (float)
- [ ] `hdcSearch` returns array of `{ id, similarity }` objects
- [ ] `.env` file contains `VITE_RPC_HTTP` and `VITE_CHAIN_ID`
- [ ] No hardcoded RPC URLs in source (all from `import.meta.env`)
- [ ] File compiles with no TypeScript errors under `strict: true`

---

## 4. ConnectionManager Class (connection.ts)

Manages the lifecycle of the RPC connection. Handles health checks, reconnection
with exponential backoff and jitter, and state transitions.

```typescript
// apps/explorer/src/data/connection.ts

import { client, getNodeStatus } from './rpc';
import { bus } from './bus';
import type { ConnectionState, NodeStatus } from './types';

// ================================================================
// CONFIGURATION
// ================================================================

const HEALTH_CHECK_INTERVAL = 5_000;      // 5s between health pings
const INITIAL_RECONNECT_DELAY = 1_000;    // 1s first retry
const MAX_RECONNECT_DELAY = 30_000;       // 30s ceiling
const RECONNECT_BACKOFF_FACTOR = 2;       // double each attempt
const JITTER_FACTOR = 0.2;               // +/- 20% randomization

// ================================================================
// CONNECTION MANAGER
// ================================================================

export class ConnectionManager {
  private state: ConnectionState = 'disconnected';
  private reconnectAttempt = 0;
  private healthCheckTimer: ReturnType<typeof setInterval> | null = null;
  private reconnectTimer: ReturnType<typeof setTimeout> | null = null;
  private destroyed = false;

  // -- Callbacks --
  onConnect: (() => void) | null = null;
  onDisconnect: (() => void) | null = null;
  onReconnect: ((attempt: number) => void) | null = null;

  // ================================================================
  // PUBLIC API
  // ================================================================

  /** Current connection state */
  getState(): ConnectionState {
    return this.state;
  }

  /** Initiate connection. Resolves when first health check passes. */
  async connect(): Promise<void> {
    if (this.destroyed) return;
    this.setState('connecting');

    try {
      // Verify connectivity with a lightweight call
      await client.getBlockNumber();
      this.setState('connected');
      this.reconnectAttempt = 0;
      this.onConnect?.();

      // Start periodic health checks
      this.startHealthCheck();
    } catch (err) {
      console.warn('[ConnectionManager] Initial connection failed:', err);
      this.scheduleReconnect();
    }
  }

  /** Gracefully shut down. No reconnection after this. */
  disconnect(): void {
    this.destroyed = true;
    this.stopHealthCheck();
    this.clearReconnectTimer();
    this.setState('disconnected');
    this.onDisconnect?.();
  }

  // ================================================================
  // HEALTH CHECK
  // ================================================================

  private startHealthCheck(): void {
    this.stopHealthCheck();

    this.healthCheckTimer = setInterval(async () => {
      if (this.destroyed) return;

      try {
        await client.getBlockNumber();

        // If we were reconnecting, we are now recovered
        if (this.state === 'reconnecting') {
          this.setState('connected');
          this.reconnectAttempt = 0;
          this.onConnect?.();
        }
      } catch {
        if (this.state === 'connected') {
          // First failure: transition to reconnecting
          this.scheduleReconnect();
        }
        // If already reconnecting, scheduleReconnect handles the loop
      }
    }, HEALTH_CHECK_INTERVAL);
  }

  private stopHealthCheck(): void {
    if (this.healthCheckTimer !== null) {
      clearInterval(this.healthCheckTimer);
      this.healthCheckTimer = null;
    }
  }

  // ================================================================
  // RECONNECTION (exponential backoff with jitter)
  // ================================================================

  private scheduleReconnect(): void {
    if (this.destroyed) return;

    this.setState('reconnecting');

    // Exponential backoff: 1s, 2s, 4s, 8s, 16s, 30s, 30s, 30s...
    const baseDelay = Math.min(
      MAX_RECONNECT_DELAY,
      INITIAL_RECONNECT_DELAY * Math.pow(RECONNECT_BACKOFF_FACTOR, this.reconnectAttempt),
    );

    // Jitter: randomize by +/- 20% to prevent thundering herd
    const jitter = baseDelay * JITTER_FACTOR * (Math.random() * 2 - 1);
    const delay = Math.max(0, Math.round(baseDelay + jitter));

    this.reconnectAttempt++;
    this.onReconnect?.(this.reconnectAttempt);

    console.info(
      `[ConnectionManager] Reconnect attempt ${this.reconnectAttempt} in ${delay}ms`,
    );

    this.clearReconnectTimer();
    this.reconnectTimer = setTimeout(async () => {
      if (this.destroyed) return;

      try {
        await client.getBlockNumber();
        this.setState('connected');
        this.reconnectAttempt = 0;
        this.onConnect?.();
        this.startHealthCheck();
      } catch {
        // Still failing: schedule next attempt
        this.scheduleReconnect();
      }
    }, delay);
  }

  private clearReconnectTimer(): void {
    if (this.reconnectTimer !== null) {
      clearTimeout(this.reconnectTimer);
      this.reconnectTimer = null;
    }
  }

  // ================================================================
  // STATE TRANSITIONS
  // ================================================================

  private setState(next: ConnectionState): void {
    if (this.state === next) return;
    const prev = this.state;
    this.state = next;

    console.debug(`[ConnectionManager] ${prev} -> ${next}`);
    bus.emit('connection:status', next);
  }
}
```

### State Machine Diagram

```
                    connect()
  disconnected ─────────────────> connecting
       ^                              │
       │                    ┌─────────┤
       │                    │ success │ failure
       │                    v         v
       │              connected   reconnecting
       │                │   ^       │   ^
       │  health fail   │   │       │   │
       │  ──────────────┘   │       │   │
       │                    │ ok    │   │ fail
       │                    └───────┘   └──── scheduleReconnect()
       │
       └──── disconnect() (from any state)
```

### Backoff Schedule Example

| Attempt | Base Delay | With Jitter (example) |
|---------|------------|----------------------|
| 1       | 1,000ms    | 800ms - 1,200ms      |
| 2       | 2,000ms    | 1,600ms - 2,400ms    |
| 3       | 4,000ms    | 3,200ms - 4,800ms    |
| 4       | 8,000ms    | 6,400ms - 9,600ms    |
| 5       | 16,000ms   | 12,800ms - 19,200ms  |
| 6+      | 30,000ms   | 24,000ms - 36,000ms  |

### Implementation Checklist -- ConnectionManager

- [ ] File created at `apps/explorer/src/data/connection.ts`
- [ ] `ConnectionState` cycles through: disconnected -> connecting -> connected
- [ ] On health check failure from `connected`: transitions to `reconnecting`
- [ ] Exponential backoff schedule: 1s, 2s, 4s, 8s, 16s, capped at 30s
- [ ] Jitter applied: each delay randomized by +/- 20%
- [ ] `reconnectAttempt` counter resets to 0 on successful reconnection
- [ ] Health check calls `eth_blockNumber` every 5 seconds
- [ ] Health check does NOT call `kora_nodeStatus` (too heavy for a ping)
- [ ] `disconnect()` sets `destroyed = true` preventing further reconnection
- [ ] `disconnect()` clears both `healthCheckTimer` and `reconnectTimer`
- [ ] `onConnect`, `onDisconnect`, `onReconnect` callbacks fire at correct transitions
- [ ] `bus.emit('connection:status', state)` fires on every state change
- [ ] Duplicate state transitions are suppressed (no emit if `prev === next`)
- [ ] No unhandled promise rejections (all `async` calls wrapped in try/catch)

---

## 5. Block Poller (poller.ts)

Polls `eth_blockNumber` at 1-second intervals. When a new block is detected,
fetches the full block and emits it to the event bus. Includes an optimistic
header-first check to skip fetching transaction bodies on empty blocks.

```typescript
// apps/explorer/src/data/poller.ts

import { client } from './rpc';
import { bus } from './bus';
import type { ChainBlock, ChainTransaction } from './types';

// ================================================================
// CONFIGURATION
// ================================================================

const BLOCK_POLL_INTERVAL = 1_000;    // 1s -- matches kora block time
const FEE_POLL_INTERVAL = 5_000;      // 5s -- fee history changes slowly

// ================================================================
// BLOCK POLLER
// ================================================================

export class BlockPoller {
  private lastBlockNumber: bigint = 0n;
  private blockTimer: ReturnType<typeof setInterval> | null = null;
  private feeTimer: ReturnType<typeof setInterval> | null = null;
  private running = false;

  // ================================================================
  // LIFECYCLE
  // ================================================================

  /** Start polling. Safe to call multiple times. */
  start(): void {
    if (this.running) return;
    this.running = true;

    // Immediate first poll, then interval
    this.pollBlock();
    this.pollFees();

    this.blockTimer = setInterval(() => this.pollBlock(), BLOCK_POLL_INTERVAL);
    this.feeTimer = setInterval(() => this.pollFees(), FEE_POLL_INTERVAL);
  }

  /** Stop all polling. */
  stop(): void {
    this.running = false;

    if (this.blockTimer !== null) {
      clearInterval(this.blockTimer);
      this.blockTimer = null;
    }
    if (this.feeTimer !== null) {
      clearInterval(this.feeTimer);
      this.feeTimer = null;
    }
  }

  /** Stop only block number polling (used when WS takes over). */
  disableBlockPoll(): void {
    if (this.blockTimer !== null) {
      clearInterval(this.blockTimer);
      this.blockTimer = null;
    }
  }

  // ================================================================
  // BLOCK POLLING
  // ================================================================

  private async pollBlock(): Promise<void> {
    try {
      const currentNumber = await client.getBlockNumber();

      // No new block: skip
      if (currentNumber <= this.lastBlockNumber) return;

      // Handle block gaps (if we missed more than one block)
      const startBlock = this.lastBlockNumber === 0n
        ? currentNumber          // first poll: just get latest
        : this.lastBlockNumber + 1n;

      for (let n = startBlock; n <= currentNumber; n++) {
        await this.fetchAndEmitBlock(n);
      }

      this.lastBlockNumber = currentNumber;
    } catch (err) {
      // Log and continue. Never break the poll loop.
      console.warn('[BlockPoller] pollBlock error:', err);
    }
  }

  /**
   * Optimistic empty-block skip:
   * 1. Fetch block header (full=false) -- lightweight, no tx bodies
   * 2. If txCount is 0, emit block without fetching full bodies
   * 3. If txCount > 0, fetch again with full=true for transaction details
   */
  private async fetchAndEmitBlock(blockNumber: bigint): Promise<void> {
    // Step 1: header-only fetch
    const header = await client.getBlock({
      blockNumber,
      includeTransactions: false,
    });

    if (!header) {
      console.warn(`[BlockPoller] Block ${blockNumber} returned null`);
      return;
    }

    const arrivalTime = Date.now();
    const txCount = header.transactions.length;

    if (txCount === 0) {
      // Step 2a: empty block -- no need for a second fetch
      const block: ChainBlock = {
        number: header.number,
        hash: header.hash,
        parentHash: header.parentHash,
        timestamp: header.timestamp,
        gasUsed: header.gasUsed,
        gasLimit: header.gasLimit,
        baseFeePerGas: header.baseFeePerGas ?? 0n,
        transactions: [],
        stateRoot: header.stateRoot,
        transactionsRoot: header.transactionsRoot,
        receiptsRoot: header.receiptsRoot,
        _activityLevel: 0,
        _arrivalTime: arrivalTime,
      };

      bus.emit('block:new', block);
      return;
    }

    // Step 2b: block has transactions -- fetch full bodies
    const fullBlock = await client.getBlock({
      blockNumber,
      includeTransactions: true,
    });

    if (!fullBlock) return;

    const activityLevel = fullBlock.gasLimit > 0n
      ? Number(fullBlock.gasUsed * 10000n / fullBlock.gasLimit) / 10000
      : 0;

    const block: ChainBlock = {
      number: fullBlock.number,
      hash: fullBlock.hash,
      parentHash: fullBlock.parentHash,
      timestamp: fullBlock.timestamp,
      gasUsed: fullBlock.gasUsed,
      gasLimit: fullBlock.gasLimit,
      baseFeePerGas: fullBlock.baseFeePerGas ?? 0n,
      transactions: fullBlock.transactions as ChainTransaction[],
      stateRoot: fullBlock.stateRoot,
      transactionsRoot: fullBlock.transactionsRoot,
      receiptsRoot: fullBlock.receiptsRoot,
      _activityLevel: activityLevel,
      _arrivalTime: arrivalTime,
    };

    bus.emit('block:new', block);

    // Emit individual transaction events
    for (const tx of block.transactions as ChainTransaction[]) {
      bus.emit('tx:confirmed', tx);
      bus.emit('address:seen', tx.from);
      if (tx.to) bus.emit('address:seen', tx.to);
    }
  }

  // ================================================================
  // FEE HISTORY POLLING
  // ================================================================

  private async pollFees(): Promise<void> {
    try {
      const history = await client.getFeeHistory({
        blockCount: 20,
        rewardPercentiles: [25, 50, 75],
      });

      if (!history || !history.baseFeePerGas) return;

      const points = history.baseFeePerGas.map((baseFee, i) => ({
        blockNumber: history.oldestBlock + BigInt(i),
        baseFee,
        gasUsedRatio: history.gasUsedRatio[i] ?? 0,
        reward25: history.reward?.[i]?.[0] ?? 0n,
        reward50: history.reward?.[i]?.[1] ?? 0n,
        reward75: history.reward?.[i]?.[2] ?? 0n,
      }));

      bus.emit('fee:update', points);
    } catch (err) {
      // Log and continue. Fee data is secondary.
      console.warn('[BlockPoller] pollFees error:', err);
    }
  }
}
```

### Poll Flow Diagram

```
every 1s
   │
   v
eth_blockNumber
   │
   ├── same as last? ──> skip
   │
   └── new number(s) detected
       │
       └── for each missed block number:
           │
           v
       eth_getBlockByNumber(n, full=false)    ← header only
           │
           ├── 0 transactions ──> emit block:new (no body fetch)
           │
           └── >0 transactions
               │
               v
           eth_getBlockByNumber(n, full=true)  ← full tx bodies
               │
               ├── emit block:new
               ├── emit tx:confirmed (each tx)
               └── emit address:seen (from, to)
```

### Implementation Checklist -- BlockPoller

- [ ] File created at `apps/explorer/src/data/poller.ts`
- [ ] `start()` begins polling at 1-second interval for blocks
- [ ] `start()` begins polling at 5-second interval for fee history
- [ ] `start()` performs an immediate first poll (does not wait for first interval tick)
- [ ] `start()` is idempotent (calling twice does not create duplicate intervals)
- [ ] `stop()` clears both block and fee timers
- [ ] `disableBlockPoll()` clears only the block timer (fee polling continues)
- [ ] Block number comparison uses `bigint` (`<=` not `<` to handle edge cases)
- [ ] Block gap handling: if `lastBlockNumber` is 5 and current is 8, fetches blocks 6, 7, 8
- [ ] First poll fetches only the latest block (not the entire chain history)
- [ ] Header-first optimization: empty blocks require only 1 RPC call (not 2)
- [ ] Blocks with transactions require 2 RPC calls (header + full body)
- [ ] `_activityLevel` computed as `gasUsed / gasLimit` with safe division (no divide-by-zero)
- [ ] `_activityLevel` uses integer arithmetic with bigint (`gasUsed * 10000n / gasLimit`) to avoid float precision issues
- [ ] `_arrivalTime` set to `Date.now()` at the moment of fetch (not block timestamp)
- [ ] `tx:confirmed` emitted for each transaction in a new block
- [ ] `address:seen` emitted for both `tx.from` and `tx.to` (skipping null `to`)
- [ ] All `pollBlock()` errors are caught and logged (poll loop never breaks)
- [ ] All `pollFees()` errors are caught and logged (poll loop never breaks)
- [ ] Fee history maps `baseFeePerGas`, `gasUsedRatio`, and reward percentiles into `FeeDataPoint[]`

---

## 6. LRU Cache (cache.ts)

Generic LRU (Least Recently Used) cache. Uses JavaScript `Map` iteration order
(insertion order) to achieve O(1) get/set/delete with automatic eviction.

```typescript
// apps/explorer/src/data/cache.ts

// ================================================================
// LRU MAP
// ================================================================

/**
 * Least Recently Used cache with O(1) operations.
 *
 * Relies on Map's insertion-order iteration:
 * - get() moves the entry to the end (most recently used)
 * - set() appends to the end
 * - eviction removes from the beginning (least recently used)
 *
 * Memory budget: ~50MB total for all caches in the explorer.
 */
export class LRUMap<K, V> {
  private map = new Map<K, V>();

  constructor(private readonly maxSize: number) {
    if (maxSize < 1) {
      throw new Error(`LRUMap maxSize must be >= 1, got ${maxSize}`);
    }
  }

  // ================================================================
  // CORE OPERATIONS
  // ================================================================

  /**
   * Retrieve a value and mark it as recently used.
   * Returns undefined if not found.
   */
  get(key: K): V | undefined {
    const value = this.map.get(key);
    if (value === undefined) return undefined;

    // Move to end (most recently used)
    this.map.delete(key);
    this.map.set(key, value);
    return value;
  }

  /**
   * Insert or update a value. Evicts the oldest entry if at capacity.
   */
  set(key: K, value: V): void {
    // If key exists, remove it first (resets position to end)
    if (this.map.has(key)) {
      this.map.delete(key);
    }

    this.map.set(key, value);

    // Evict oldest if over capacity
    if (this.map.size > this.maxSize) {
      const oldestKey = this.map.keys().next().value;
      if (oldestKey !== undefined) {
        this.map.delete(oldestKey);
      }
    }
  }

  /**
   * Check if key exists. Does NOT mark as recently used.
   */
  has(key: K): boolean {
    return this.map.has(key);
  }

  /**
   * Remove an entry. Returns true if the entry existed.
   */
  delete(key: K): boolean {
    return this.map.delete(key);
  }

  // ================================================================
  // INSPECTION
  // ================================================================

  /** Current number of entries. */
  get size(): number {
    return this.map.size;
  }

  /** Maximum capacity. */
  get capacity(): number {
    return this.maxSize;
  }

  /** Remove all entries. */
  clear(): void {
    this.map.clear();
  }

  // ================================================================
  // ITERATION
  // ================================================================

  /**
   * Iterate values from newest to oldest.
   * Useful for rendering recent blocks first.
   */
  *values(): IterableIterator<V> {
    const entries = [...this.map.values()];
    for (let i = entries.length - 1; i >= 0; i--) {
      yield entries[i];
    }
  }

  /**
   * Iterate entries from newest to oldest.
   */
  *entries(): IterableIterator<[K, V]> {
    const entries = [...this.map.entries()];
    for (let i = entries.length - 1; i >= 0; i--) {
      yield entries[i];
    }
  }

  /**
   * Iterate keys from newest to oldest.
   */
  *keys(): IterableIterator<K> {
    const keys = [...this.map.keys()];
    for (let i = keys.length - 1; i >= 0; i--) {
      yield keys[i];
    }
  }
}

// ================================================================
// PRE-CONFIGURED CACHES FOR THE EXPLORER
// ================================================================

import type { ChainBlock, ChainTransaction, ChainReceipt } from './types';

/** Block cache: 500 most recent blocks (~5MB at ~10KB/block) */
export const blockCache = new LRUMap<bigint, ChainBlock>(500);

/** Transaction cache: 2000 most recent transactions */
export const txCache = new LRUMap<`0x${string}`, ChainTransaction>(2000);

/** Receipt cache: 1000 most recent receipts */
export const receiptCache = new LRUMap<`0x${string}`, ChainReceipt>(1000);
```

### Memory Budget Breakdown

| Cache Instance  | Max Entries | Est. Entry Size | Est. Total |
|-----------------|-------------|-----------------|------------|
| `blockCache`    | 500         | ~10KB           | ~5MB       |
| `txCache`       | 2,000       | ~500B           | ~1MB       |
| `receiptCache`  | 1,000       | ~2KB            | ~2MB       |
| Address map     | unbounded   | ~120B           | ~12MB @ 100K |
| Fee ring buffer | 1,000       | ~64B            | ~64KB      |
| **JS heap total** |           |                 | **~20MB typical** |

The 50MB total memory budget from the design spec is conservative. Actual JS heap
usage is lower because bigints are 8 bytes (not hex strings), and LRU eviction
is aggressive. GPU memory for WebGL scenes dominates the real memory footprint.

### Implementation Checklist -- LRU Cache

- [ ] File created at `apps/explorer/src/data/cache.ts`
- [ ] `LRUMap` is generic: `LRUMap<K, V>` with configurable `maxSize`
- [ ] Constructor throws if `maxSize < 1`
- [ ] `get(key)` returns `undefined` for missing keys (not `null`)
- [ ] `get(key)` moves accessed entry to end of Map (most recently used)
- [ ] `set(key, value)` adds entry at end of Map
- [ ] `set(key, value)` on existing key: deletes old position, re-inserts at end
- [ ] When `map.size > maxSize` after `set()`: first (oldest) key is deleted
- [ ] `has(key)` does NOT change entry position (read-only check)
- [ ] `delete(key)` returns `boolean` matching `Map.delete` semantics
- [ ] `size` getter returns current entry count
- [ ] `capacity` getter returns the configured `maxSize`
- [ ] `clear()` removes all entries
- [ ] `values()` iterates from newest to oldest (reversed iteration)
- [ ] `entries()` iterates from newest to oldest
- [ ] `keys()` iterates from newest to oldest
- [ ] `blockCache` pre-instantiated with capacity 500
- [ ] `txCache` pre-instantiated with capacity 2,000
- [ ] `receiptCache` pre-instantiated with capacity 1,000
- [ ] Verify: insert 501 entries into `blockCache`, `size` is 500, entry #1 is evicted
- [ ] Verify: `get()` on entry #2 after inserting 500 entries keeps entry #2 alive (not evicted)

---

## 7. Integration Wiring

How the pieces connect in `main.tsx`:

```typescript
// apps/explorer/src/main.tsx (data layer initialization excerpt)

import { ConnectionManager } from './data/connection';
import { BlockPoller } from './data/poller';
import { bus } from './data/bus';
import { blockCache } from './data/cache';
import { useChainStore } from './data/store';

// 1. Wire EventBus -> Zustand store
bus.on('block:new', (block) => {
  // Cache the block
  blockCache.set(block.number, block);

  // Update Zustand store (triggers React re-renders in panels)
  useChainStore.getState().addBlock(block);

  // Update atmosphere CSS variable for rose wash
  document.documentElement.style.setProperty(
    '--rd-activity',
    String(block._activityLevel * 0.15),
  );
});

bus.on('connection:status', (status) => {
  useChainStore.getState().setConnectionStatus(status);
});

// 2. Create connection manager and poller
const connectionManager = new ConnectionManager();
const poller = new BlockPoller();

// 3. Wire connection lifecycle to poller
connectionManager.onConnect = () => {
  poller.start();
};
connectionManager.onDisconnect = () => {
  poller.stop();
};

// 4. Start
connectionManager.connect();
```

### Data Flow Summary

```
kora node (port 8545)
    |
    | HTTP JSON-RPC
    |
viem client (rpc.ts)
    |
    +-- ConnectionManager (connection.ts)
    |     |
    |     +-- health check every 5s via eth_blockNumber
    |     +-- exponential backoff on failure
    |     +-- emits connection:status to bus
    |
    +-- BlockPoller (poller.ts)
          |
          +-- eth_blockNumber every 1s
          +-- eth_getBlockByNumber (header-first optimization)
          +-- eth_feeHistory every 5s
          |
          +-- emits to EventBus:
                |
                +-- block:new      -> blockCache.set() + Zustand store + scenes
                +-- tx:confirmed   -> txCache.set() + constellation scene
                +-- address:seen   -> address map + constellation nodes
                +-- fee:update     -> fee ring buffer + waveform panel
```

---

## 8. Full Verification Checklist

Run these checks against a live kora node (local or remote).

### viem Client Connectivity

- [ ] `client.getBlockNumber()` returns a `bigint` greater than `0n`
- [ ] `client.getBlock({ blockNumber: 1n })` returns a block object (not null)
- [ ] `client.getBlock({ blockNumber: 1n, includeTransactions: true })` includes `transactions` array
- [ ] `client.getBalance({ address: '0x...' })` returns a `bigint` (even if 0)
- [ ] `client.getGasPrice()` returns a `bigint`
- [ ] `client.getFeeHistory({ blockCount: 10, rewardPercentiles: [25, 50, 75] })` returns `baseFeePerGas` array

### kora_nodeStatus

- [ ] `getNodeStatus()` resolves without error
- [ ] Result contains `chainId` as a number (e.g., 1337)
- [ ] Result contains `validatorIndex` as a number (0-based)
- [ ] Result contains `currentView` as a number
- [ ] Result contains `finalizedCount` as a number
- [ ] Result contains `proposedCount` as a number
- [ ] Result contains `nullifiedCount` as a number
- [ ] Result contains `peerCount` as a number (>= 0)
- [ ] Result contains `isLeader` as a boolean

### ConnectionManager Behavior

- [ ] On fresh `connect()`: state transitions disconnected -> connecting -> connected
- [ ] After `connect()`: `bus` receives `'connection:status'` event with `'connected'`
- [ ] Stop the kora node: within 5s, state transitions to `reconnecting`
- [ ] During reconnecting: backoff delay increases (check console logs for delay values)
- [ ] Restart the kora node: state transitions back to `connected`
- [ ] `reconnectAttempt` resets to 0 after successful reconnection
- [ ] Call `disconnect()`: state transitions to `disconnected`, no further reconnection attempts
- [ ] Jitter is observable: two consecutive reconnect delays are not identical

### Block Poller

- [ ] After `start()`: `block:new` event fires within 2 seconds
- [ ] Block number in emitted block matches `eth_blockNumber` response
- [ ] Empty blocks (0 transactions) emit with `_activityLevel: 0`
- [ ] Empty blocks require exactly 1 RPC call (header only, verify in Network tab)
- [ ] Blocks with transactions require 2 RPC calls (header + full, verify in Network tab)
- [ ] `tx:confirmed` fires once per transaction in a new block
- [ ] `address:seen` fires for `from` address of every transaction
- [ ] `address:seen` fires for `to` address when `to` is not null
- [ ] `fee:update` fires every 5 seconds with an array of `FeeDataPoint`
- [ ] Stopping the kora node does not crash the poller (errors logged, loop continues)
- [ ] Calling `stop()` ceases all polling (no further events emitted)

### LRU Cache

- [ ] Create `new LRUMap<number, string>(3)`
- [ ] Insert entries 1, 2, 3: `size` is 3
- [ ] Insert entry 4: `size` is still 3, entry 1 is evicted (`has(1)` returns false)
- [ ] Call `get(2)` then insert entry 5: entry 3 is evicted (not entry 2, because `get` refreshed it)
- [ ] `values()` yields entries in newest-to-oldest order
- [ ] `delete(key)` returns `true` for existing key, `false` for missing key
- [ ] `clear()` reduces `size` to 0
- [ ] `blockCache.capacity` is 500
- [ ] `txCache.capacity` is 2,000
- [ ] `receiptCache.capacity` is 1,000

### End-to-End Integration

- [ ] App starts, ConnectionManager connects, BlockPoller begins
- [ ] New blocks appear in Zustand store (`useChainStore.getState().latestBlock` is not null)
- [ ] `--rd-activity` CSS variable updates when blocks with transactions arrive
- [ ] `blockCache.size` grows as new blocks arrive
- [ ] After 500+ blocks, `blockCache.size` stays at 500 (eviction works)
- [ ] Disconnecting the node: UI shows `reconnecting` state
- [ ] Reconnecting the node: UI shows `connected` state, new blocks resume
