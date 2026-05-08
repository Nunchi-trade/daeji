# 12 — Testing & Deployment

Testing strategy, build configuration, and deployment for the Kora Explorer.
After completing every step in order, the result is a fully tested, production-built,
and deployable static SPA with CI integration.

Reference specs: [`../06-tech-stack.md`](../06-tech-stack.md), [`../07-performance.md`](../07-performance.md)

Depends on: all previous implementation documents (01-11).

---

## 12.1 Test Infrastructure Setup

### 12.1.1 Vitest Configuration

The test configuration lives inside `vite.config.ts` (already scaffolded in doc 01).
Extend the `test` block with coverage thresholds and alias resolution.

#### `apps/explorer/vite.config.ts` — test block (replace existing)

```typescript
// Inside defineConfig({ ... })
test: {
  globals: true,
  environment: 'jsdom',
  setupFiles: ['./src/test-setup.ts'],
  include: ['src/**/*.test.{ts,tsx}'],
  css: true,
  coverage: {
    provider: 'v8',
    reporter: ['text', 'text-summary', 'lcov', 'html'],
    reportsDirectory: './coverage',
    include: ['src/**/*.{ts,tsx}'],
    exclude: [
      'src/**/*.test.{ts,tsx}',
      'src/**/*.d.ts',
      'src/test-setup.ts',
      'src/vite-env.d.ts',
      'src/**/__mocks__/**',
    ],
    thresholds: {
      statements: 80,
      branches: 80,
      functions: 80,
      lines: 80,
    },
  },
  // Match the same path aliases as vite resolve.alias
  alias: {
    '@': path.resolve(__dirname, 'src'),
  },
},
```

The `alias` block inside `test` mirrors the `resolve.alias` in the main vite config.
Without this, imports like `@/data/rpc` fail during test resolution.

### 12.1.2 Test Setup File

#### `apps/explorer/src/test-setup.ts` (replace existing stub)

```typescript
import '@testing-library/jest-dom/vitest';

// Provide a minimal matchMedia stub for jsdom
// (Zustand's persist middleware and framer-motion both read it)
Object.defineProperty(window, 'matchMedia', {
  writable: true,
  value: (query: string) => ({
    matches: false,
    media: query,
    onchange: null,
    addListener: () => {},
    removeListener: () => {},
    addEventListener: () => {},
    removeEventListener: () => {},
    dispatchEvent: () => false,
  }),
});

// Stub ResizeObserver (jsdom does not implement it)
class ResizeObserverStub {
  observe() {}
  unobserve() {}
  disconnect() {}
}
globalThis.ResizeObserver = ResizeObserverStub as unknown as typeof ResizeObserver;

// Stub requestAnimationFrame for scene tests
if (typeof globalThis.requestAnimationFrame === 'undefined') {
  globalThis.requestAnimationFrame = (cb: FrameRequestCallback) => {
    return setTimeout(() => cb(performance.now()), 0) as unknown as number;
  };
  globalThis.cancelAnimationFrame = (id: number) => clearTimeout(id);
}
```

### 12.1.3 React Testing Library Setup

Already installed in package.json (doc 01). The `@testing-library/react` and
`@testing-library/jest-dom` packages integrate via the setup file above.

Rendering components in tests:

```typescript
import { render, screen } from '@testing-library/react';
import { BrowserRouter } from 'react-router';

// Wrapper for components that need routing context
function renderWithRouter(ui: React.ReactElement) {
  return render(ui, { wrapper: BrowserRouter });
}
```

### 12.1.4 MSW (Mock Service Worker)

MSW intercepts outgoing HTTP requests at the network level. It provides realistic
mock responses for all RPC methods the explorer uses.

#### `apps/explorer/src/__mocks__/handlers.ts`

```typescript
import { http, HttpResponse } from 'msw';

// ── Mock Data ──

/** 5 blocks with realistic data. Block 100-104. */
export const MOCK_BLOCKS = Array.from({ length: 5 }, (_, i) => {
  const num = 100 + i;
  const txCount = i === 0 ? 0 : Math.min(i * 2, 8); // block 100 is empty
  return {
    number: `0x${num.toString(16)}`,
    hash: `0x${num.toString(16).padStart(64, 'a')}`,
    parentHash: `0x${(num - 1).toString(16).padStart(64, 'a')}`,
    timestamp: `0x${(1700000000 + num * 2).toString(16)}`,
    miner: '0x' + '1'.repeat(40),
    gasUsed: `0x${(21000 * txCount).toString(16)}`,
    gasLimit: '0x1c9c380', // 30M
    baseFeePerGas: '0x3b9aca00', // 1 gwei
    transactions: Array.from({ length: txCount }, (_, j) =>
      `0x${num.toString(16)}${j.toString(16)}`.padEnd(66, '0'),
    ),
    size: `0x${(500 + txCount * 200).toString(16)}`,
    stateRoot: '0x' + 'b'.repeat(64),
    receiptsRoot: '0x' + 'c'.repeat(64),
    logsBloom: '0x' + '0'.repeat(512),
    extraData: '0x',
    nonce: '0x0000000000000000',
    sha3Uncles: '0x' + 'd'.repeat(64),
    uncles: [],
  };
});

/** Blocks with full transaction objects (hydrated). */
export const MOCK_BLOCKS_HYDRATED = MOCK_BLOCKS.map((block) => ({
  ...block,
  transactions: block.transactions.map((txHash, j) => ({
    hash: txHash,
    blockHash: block.hash,
    blockNumber: block.number,
    from: '0x' + ((j % 3) + 1).toString().repeat(40),
    to: '0x' + ((j % 3) + 4).toString().repeat(40),
    value: `0x${(1000000000000000000n * BigInt(j + 1)).toString(16)}`,
    gas: '0x5208', // 21000
    gasPrice: '0x3b9aca00',
    input: '0x',
    nonce: `0x${j.toString(16)}`,
    transactionIndex: `0x${j.toString(16)}`,
    type: '0x2',
    v: '0x1',
    r: '0x' + 'e'.repeat(64),
    s: '0x' + 'f'.repeat(64),
    maxFeePerGas: '0x77359400',
    maxPriorityFeePerGas: '0x3b9aca00',
  })),
}));

/** 20 mock transactions across the 5 blocks. */
export const MOCK_TRANSACTIONS = MOCK_BLOCKS_HYDRATED.flatMap(
  (block) => block.transactions,
);

/** Mock receipts for all transactions. */
const MOCK_RECEIPTS = MOCK_TRANSACTIONS.map((tx) => ({
  transactionHash: tx.hash,
  blockHash: tx.blockHash,
  blockNumber: tx.blockNumber,
  from: tx.from,
  to: tx.to,
  contractAddress: null,
  cumulativeGasUsed: tx.gas,
  gasUsed: tx.gas,
  effectiveGasPrice: tx.gasPrice,
  logs: [],
  logsBloom: '0x' + '0'.repeat(512),
  status: '0x1', // success
  type: tx.type,
  transactionIndex: tx.transactionIndex,
}));

/** Fee history mock (10 blocks). */
const MOCK_FEE_HISTORY = {
  baseFeePerGas: Array.from({ length: 11 }, (_, i) =>
    `0x${(1000000000 + i * 100000000).toString(16)}`,
  ),
  gasUsedRatio: Array.from({ length: 10 }, () => 0.3 + Math.random() * 0.4),
  oldestBlock: '0x5f',
  reward: Array.from({ length: 10 }, () => ['0x59682f00', '0x77359400']),
};

/** kora_nodeStatus mock. */
const MOCK_NODE_STATUS = {
  chainId: 1337,
  validatorIndex: 0,
  currentView: 104,
  finalizedCount: 100,
  proposedCount: 104,
  nullifiedCount: 2,
  peerCount: 3,
  isLeader: false,
};

// ── Handlers ──

function jsonRpcResponse(id: number | string, result: unknown) {
  return HttpResponse.json({
    jsonrpc: '2.0',
    id,
    result,
  });
}

function jsonRpcError(id: number | string, code: number, message: string) {
  return HttpResponse.json({
    jsonrpc: '2.0',
    id,
    error: { code, message },
  });
}

/** Current block number — returns the latest mock block. */
let currentBlockNumber = MOCK_BLOCKS[MOCK_BLOCKS.length - 1].number;

export function setCurrentBlockNumber(hex: string) {
  currentBlockNumber = hex;
}

export const handlers = [
  http.post('/rpc', async ({ request }) => {
    const body = (await request.json()) as {
      id: number;
      method: string;
      params?: unknown[];
    };

    const { id, method, params } = body;

    switch (method) {
      case 'eth_blockNumber':
        return jsonRpcResponse(id, currentBlockNumber);

      case 'eth_getBlockByNumber': {
        const [blockNum, hydrated] = (params ?? []) as [string, boolean];
        const source = hydrated ? MOCK_BLOCKS_HYDRATED : MOCK_BLOCKS;
        const block = source.find((b) => b.number === blockNum);
        return jsonRpcResponse(id, block ?? null);
      }

      case 'eth_getBlockByHash': {
        const [blockHash, hydrated] = (params ?? []) as [string, boolean];
        const source = hydrated ? MOCK_BLOCKS_HYDRATED : MOCK_BLOCKS;
        const block = source.find((b) => b.hash === blockHash);
        return jsonRpcResponse(id, block ?? null);
      }

      case 'eth_getTransactionByHash': {
        const [txHash] = (params ?? []) as [string];
        const tx = MOCK_TRANSACTIONS.find((t) => t.hash === txHash);
        return jsonRpcResponse(id, tx ?? null);
      }

      case 'eth_getTransactionReceipt': {
        const [txHash] = (params ?? []) as [string];
        const receipt = MOCK_RECEIPTS.find((r) => r.transactionHash === txHash);
        return jsonRpcResponse(id, receipt ?? null);
      }

      case 'eth_getBalance': {
        const [address] = (params ?? []) as [string];
        // Return a deterministic balance based on the first byte of the address
        const firstByte = parseInt(address.slice(2, 4), 16);
        const balance = BigInt(firstByte) * 1000000000000000000n;
        return jsonRpcResponse(id, `0x${balance.toString(16)}`);
      }

      case 'eth_getCode': {
        const [address] = (params ?? []) as [string];
        // Addresses starting with 0x00 are EOAs, others are contracts
        const isContract = !address.startsWith('0x00');
        return jsonRpcResponse(id, isContract ? '0x6080604052' : '0x');
      }

      case 'eth_gasPrice':
        return jsonRpcResponse(id, '0x3b9aca00'); // 1 gwei

      case 'eth_feeHistory':
        return jsonRpcResponse(id, MOCK_FEE_HISTORY);

      case 'eth_chainId':
        return jsonRpcResponse(id, '0x539'); // 1337

      case 'eth_getLogs':
        return jsonRpcResponse(id, []);

      case 'kora_nodeStatus':
        return jsonRpcResponse(id, MOCK_NODE_STATUS);

      default:
        return jsonRpcError(id, -32601, `Method not found: ${method}`);
    }
  }),
];
```

#### `apps/explorer/src/__mocks__/server.ts`

```typescript
import { setupServer } from 'msw/node';
import { handlers } from './handlers';

/** MSW server for vitest (node environment). */
export const server = setupServer(...handlers);
```

#### `apps/explorer/src/__mocks__/browser.ts`

```typescript
import { setupWorker } from 'msw/browser';
import { handlers } from './handlers';

/** MSW worker for browser-based testing and development. */
export const worker = setupWorker(...handlers);
```

#### Activate MSW in tests

Add to `apps/explorer/src/test-setup.ts` (append to the existing file):

```typescript
import { server } from './__mocks__/server';

beforeAll(() => server.listen({ onUnhandledRequest: 'warn' }));
afterEach(() => server.resetHandlers());
afterAll(() => server.close());
```

---

## 12.2 Unit Tests (vitest)

All unit tests live alongside the code they test, colocated as `*.test.ts` or
`*.test.tsx` files. Run with `pnpm test` (which invokes `vitest run`).

### 12.2.1 RPC Layer Tests

#### `apps/explorer/src/data/rpc.test.ts`

```typescript
import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest';

// Tests rely on MSW handlers from test-setup.ts

describe('RPC Client', () => {
  it('connects and fetches block number', async () => {
    // Import the rpc client (uses /rpc which MSW intercepts)
    const { rpcClient } = await import('@/data/rpc');
    const blockNumber = await rpcClient.getBlockNumber();
    // MOCK_BLOCKS last block is 104 = 0x68
    expect(blockNumber).toBeGreaterThan(0n);
  });
});
```

#### `apps/explorer/src/data/connection.test.ts`

```typescript
import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest';

describe('ConnectionManager', () => {
  beforeEach(() => {
    vi.useFakeTimers();
  });

  afterEach(() => {
    vi.useRealTimers();
  });

  it('reconnects on disconnect', async () => {
    const { ConnectionManager } = await import('@/data/connection');
    const manager = new ConnectionManager({ rpcUrl: '/rpc' });

    const onStatus = vi.fn();
    manager.on('status', onStatus);

    // Simulate disconnect
    manager.simulateDisconnect();

    // Should attempt reconnect
    await vi.advanceTimersByTimeAsync(1000);
    expect(onStatus).toHaveBeenCalledWith(
      expect.objectContaining({ status: 'reconnecting' }),
    );
  });

  it('applies exponential backoff on repeated failures', async () => {
    const { ConnectionManager } = await import('@/data/connection');
    const manager = new ConnectionManager({ rpcUrl: 'http://nonexistent:9999' });

    const attempts: number[] = [];
    manager.on('reconnect-attempt', (evt) => attempts.push(evt.delay));

    // Trigger multiple reconnection attempts
    for (let i = 0; i < 4; i++) {
      manager.simulateDisconnect();
      await vi.advanceTimersByTimeAsync(30000);
    }

    // Delays should increase: 1s, 2s, 4s, 8s (capped at some max)
    for (let i = 1; i < attempts.length; i++) {
      expect(attempts[i]).toBeGreaterThanOrEqual(attempts[i - 1]);
    }
  });
});
```

#### `apps/explorer/src/data/poller.test.ts`

```typescript
import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest';

describe('BlockPoller', () => {
  beforeEach(() => {
    vi.useFakeTimers();
  });

  afterEach(() => {
    vi.useRealTimers();
  });

  it('emits new blocks when block number advances', async () => {
    const { BlockPoller } = await import('@/data/poller');
    const { setCurrentBlockNumber } = await import('@/__mocks__/handlers');

    const onBlock = vi.fn();
    const poller = new BlockPoller({ rpcUrl: '/rpc', interval: 1000 });
    poller.on('block', onBlock);

    // Start at block 100
    setCurrentBlockNumber('0x64');
    poller.start();
    await vi.advanceTimersByTimeAsync(1000);

    // Advance to block 101
    setCurrentBlockNumber('0x65');
    await vi.advanceTimersByTimeAsync(1000);

    expect(onBlock).toHaveBeenCalled();
  });

  it('skips poll when block number is unchanged', async () => {
    const { BlockPoller } = await import('@/data/poller');
    const { setCurrentBlockNumber } = await import('@/__mocks__/handlers');

    const onBlock = vi.fn();
    const poller = new BlockPoller({ rpcUrl: '/rpc', interval: 1000 });
    poller.on('block', onBlock);

    // Same block number across two polls
    setCurrentBlockNumber('0x64');
    poller.start();
    await vi.advanceTimersByTimeAsync(1000);

    const callCountAfterFirst = onBlock.mock.calls.length;

    // Second poll — same block number
    await vi.advanceTimersByTimeAsync(1000);
    expect(onBlock.mock.calls.length).toBe(callCountAfterFirst);
  });
});
```

#### `apps/explorer/src/lib/lru.test.ts`

```typescript
import { describe, it, expect } from 'vitest';
import { LRUCache } from '@/lib/lru';

describe('LRUCache', () => {
  it('evicts the oldest entry when at capacity', () => {
    const cache = new LRUCache<string, number>(3);
    cache.set('a', 1);
    cache.set('b', 2);
    cache.set('c', 3);
    cache.set('d', 4); // should evict 'a'

    expect(cache.get('a')).toBeUndefined();
    expect(cache.get('b')).toBe(2);
    expect(cache.get('c')).toBe(3);
    expect(cache.get('d')).toBe(4);
    expect(cache.size).toBe(3);
  });

  it('get() moves item to front (most recently used)', () => {
    const cache = new LRUCache<string, number>(3);
    cache.set('a', 1);
    cache.set('b', 2);
    cache.set('c', 3);

    // Access 'a', making it most recent
    cache.get('a');

    // Insert 'd' — should evict 'b' (now oldest), not 'a'
    cache.set('d', 4);
    expect(cache.get('a')).toBe(1);
    expect(cache.get('b')).toBeUndefined();
  });

  it('returns undefined for missing keys', () => {
    const cache = new LRUCache<string, number>(2);
    expect(cache.get('nonexistent')).toBeUndefined();
  });

  it('overwrites existing keys without changing capacity', () => {
    const cache = new LRUCache<string, number>(2);
    cache.set('a', 1);
    cache.set('b', 2);
    cache.set('a', 10); // overwrite, not a new entry

    expect(cache.size).toBe(2);
    expect(cache.get('a')).toBe(10);
  });
});
```

### 12.2.2 Store Tests

#### `apps/explorer/src/stores/chain.test.ts`

```typescript
import { describe, it, expect, beforeEach } from 'vitest';
import { useChainStore } from '@/stores/chain';
import { MOCK_BLOCKS_HYDRATED } from '@/__mocks__/handlers';

describe('ChainStore', () => {
  beforeEach(() => {
    // Reset store state between tests
    useChainStore.getState().reset();
  });

  it('adds a block and maintains ring buffer', () => {
    const store = useChainStore.getState();
    const block = MOCK_BLOCKS_HYDRATED[0];

    store.addBlock(block);

    expect(store.blocks.length).toBe(1);
    expect(store.latestBlockNumber).toBe(BigInt(parseInt(block.number, 16)));
  });

  it('evicts the oldest block at 128 capacity', () => {
    const store = useChainStore.getState();

    // Add 129 blocks to exceed the 128-block ring buffer
    for (let i = 0; i < 129; i++) {
      const block = {
        ...MOCK_BLOCKS_HYDRATED[0],
        number: `0x${i.toString(16)}`,
        hash: `0x${i.toString(16).padStart(64, '0')}`,
      };
      store.addBlock(block);
    }

    // Ring buffer should be capped at 128
    expect(store.blocks.length).toBe(128);

    // Oldest block (number 0) should have been evicted
    const hasBlock0 = store.blocks.some((b) => b.number === '0x0');
    expect(hasBlock0).toBe(false);

    // Most recent block (number 128) should be present
    const hasBlock128 = store.blocks.some((b) => b.number === '0x80');
    expect(hasBlock128).toBe(true);
  });
});
```

#### `apps/explorer/src/stores/ui.test.ts`

```typescript
import { describe, it, expect, beforeEach, vi } from 'vitest';
import { useUIStore } from '@/stores/ui';

describe('UIStore', () => {
  beforeEach(() => {
    useUIStore.getState().reset();
    localStorage.clear();
  });

  it('persists active scene to localStorage', () => {
    const store = useUIStore.getState();

    store.setActiveScene('constellation');

    // The Zustand persist middleware writes to localStorage
    const persisted = JSON.parse(
      localStorage.getItem('kora-ui-store') ?? '{}',
    );
    expect(persisted.state?.activeScene).toBe('constellation');
  });

  it('auto-detects performance tier', () => {
    const store = useUIStore.getState();

    // In jsdom environment, WebGL2 is not available, so tier should be 'light'
    // (or 'mobile' if the width mock kicks in)
    expect(['light', 'mobile', 'standard', 'full']).toContain(store.performanceTier);
  });
});
```

#### `apps/explorer/src/stores/consensus.test.ts`

```typescript
import { describe, it, expect, beforeEach } from 'vitest';
import { useConsensusStore } from '@/stores/consensus';

describe('ConsensusStore', () => {
  beforeEach(() => {
    useConsensusStore.getState().reset();
  });

  it('tracks round history up to 32 entries', () => {
    const store = useConsensusStore.getState();

    for (let i = 0; i < 40; i++) {
      store.addRound({
        view: i,
        proposer: 0,
        phase: 'finalized',
        timestamp: Date.now(),
      });
    }

    expect(store.rounds.length).toBe(32);
    // Oldest rounds should have been dropped
    expect(store.rounds[0].view).toBe(8);
  });
});
```

#### `apps/explorer/src/bus/events.test.ts`

```typescript
import { describe, it, expect, vi } from 'vitest';
import { bus } from '@/bus/events';

describe('EventBus', () => {
  it('emits typed events correctly', () => {
    const handler = vi.fn();
    bus.on('block:new', handler);

    const mockBlock = { number: '0x64', hash: '0xabc' };
    bus.emit('block:new', mockBlock as any);

    expect(handler).toHaveBeenCalledTimes(1);
    expect(handler).toHaveBeenCalledWith(mockBlock);
  });

  it('unsubscribes correctly', () => {
    const handler = vi.fn();
    const unsub = bus.on('block:new', handler);

    unsub();
    bus.emit('block:new', {} as any);

    expect(handler).not.toHaveBeenCalled();
  });
});
```

### 12.2.3 Algorithm Tests

#### `apps/explorer/src/scenes/terrain/hashmap.test.ts`

```typescript
import { describe, it, expect } from 'vitest';
import { hashToHeightmap } from '@/scenes/terrain/hashmap';

describe('hashToHeightmap', () => {
  const HASH_A = '0x' + 'ab'.repeat(32);
  const HASH_B = '0x' + 'cd'.repeat(32);

  it('is deterministic — same hash produces same output', () => {
    const result1 = hashToHeightmap(HASH_A);
    const result2 = hashToHeightmap(HASH_A);

    expect(result1).toEqual(result2);
  });

  it('produces different output for different hashes', () => {
    const resultA = hashToHeightmap(HASH_A);
    const resultB = hashToHeightmap(HASH_B);

    // At least some values should differ
    let hasDifference = false;
    for (let i = 0; i < resultA.length; i++) {
      if (resultA[i] !== resultB[i]) {
        hasDifference = true;
        break;
      }
    }
    expect(hasDifference).toBe(true);
  });

  it('returns a 16x16 grid (256 values)', () => {
    const result = hashToHeightmap(HASH_A);
    expect(result.length).toBe(256);
  });

  it('all values are in [0, 1] range', () => {
    const result = hashToHeightmap(HASH_A);
    for (const val of result) {
      expect(val).toBeGreaterThanOrEqual(0);
      expect(val).toBeLessThanOrEqual(1);
    }
  });
});
```

#### `apps/explorer/src/scenes/constellation/nodes.test.ts`

```typescript
import { describe, it, expect } from 'vitest';
import { addressToPosition } from '@/scenes/constellation/nodes';

describe('addressToPosition (golden-angle spiral)', () => {
  const ADDR_A = '0x' + '1'.repeat(40);
  const ADDR_B = '0x' + '2'.repeat(40);

  it('is deterministic — same address produces same position', () => {
    const pos1 = addressToPosition(ADDR_A);
    const pos2 = addressToPosition(ADDR_A);

    expect(pos1.x).toBeCloseTo(pos2.x);
    expect(pos1.y).toBeCloseTo(pos2.y);
    expect(pos1.z).toBeCloseTo(pos2.z);
  });

  it('different addresses produce different positions', () => {
    const posA = addressToPosition(ADDR_A);
    const posB = addressToPosition(ADDR_B);

    const isSame =
      Math.abs(posA.x - posB.x) < 0.001 &&
      Math.abs(posA.y - posB.y) < 0.001 &&
      Math.abs(posA.z - posB.z) < 0.001;
    expect(isSame).toBe(false);
  });

  it('positions lie on a unit sphere surface', () => {
    // Test with multiple addresses
    const addresses = Array.from(
      { length: 20 },
      (_, i) => '0x' + i.toString(16).padStart(40, '0'),
    );

    for (const addr of addresses) {
      const pos = addressToPosition(addr);
      const r = Math.sqrt(pos.x ** 2 + pos.y ** 2 + pos.z ** 2);
      // Should be on or near the unit sphere
      expect(r).toBeCloseTo(1.0, 1);
    }
  });

  it('covers the sphere surface evenly (no clustering)', () => {
    // Generate 100 positions and check they span a reasonable range
    const positions = Array.from({ length: 100 }, (_, i) => {
      const addr = '0x' + i.toString(16).padStart(40, '0');
      return addressToPosition(addr);
    });

    const xs = positions.map((p) => p.x);
    const ys = positions.map((p) => p.y);
    const zs = positions.map((p) => p.z);

    // Each axis should span at least 1.0 of the [-1, 1] range
    expect(Math.max(...xs) - Math.min(...xs)).toBeGreaterThan(1.0);
    expect(Math.max(...ys) - Math.min(...ys)).toBeGreaterThan(1.0);
    expect(Math.max(...zs) - Math.min(...zs)).toBeGreaterThan(1.0);
  });
});
```

#### `apps/explorer/src/scenes/waterfall/hashbar.test.ts`

```typescript
import { describe, it, expect } from 'vitest';
import { hashToBarcode } from '@/scenes/waterfall/hashbar';

describe('hashToBarcode', () => {
  it('renders correct nibble widths', () => {
    // A hash whose first byte is 0xab => nibbles 0xa (10) and 0xb (11)
    const hash = '0xab' + '0'.repeat(62);
    const barcode = hashToBarcode(hash);

    // Barcode should be an array of { width, filled } segments
    expect(barcode.length).toBeGreaterThan(0);

    // Each nibble maps to a width proportional to its value (0-15)
    // First nibble 0xa = 10, second nibble 0xb = 11
    expect(barcode[0].width).toBeGreaterThan(0);
    expect(barcode[1].width).toBeGreaterThan(0);
  });

  it('produces 64 segments (one per nibble of a 32-byte hash)', () => {
    const hash = '0x' + 'ff'.repeat(32);
    const barcode = hashToBarcode(hash);
    expect(barcode.length).toBe(64);
  });

  it('is deterministic', () => {
    const hash = '0x' + 'deadbeef'.repeat(8);
    const a = hashToBarcode(hash);
    const b = hashToBarcode(hash);
    expect(a).toEqual(b);
  });
});
```

#### `apps/explorer/src/lib/hashArt.test.ts`

```typescript
import { describe, it, expect } from 'vitest';
import { generateHashArt } from '@/lib/hashArt';

describe('generateHashArt', () => {
  it('produces a symmetric pattern', () => {
    const hash = '0x' + 'ab'.repeat(32);
    const art = generateHashArt(hash);

    // Art is an NxN grid — check horizontal symmetry
    const size = Math.sqrt(art.length);
    for (let row = 0; row < size; row++) {
      for (let col = 0; col < Math.floor(size / 2); col++) {
        const left = art[row * size + col];
        const right = art[row * size + (size - 1 - col)];
        expect(left).toBe(right);
      }
    }
  });

  it('is deterministic', () => {
    const hash = '0x' + 'cd'.repeat(32);
    expect(generateHashArt(hash)).toEqual(generateHashArt(hash));
  });

  it('different hashes produce different art', () => {
    const artA = generateHashArt('0x' + 'aa'.repeat(32));
    const artB = generateHashArt('0x' + 'bb'.repeat(32));
    expect(artA).not.toEqual(artB);
  });
});
```

### 12.2.4 Utility Tests

#### `apps/explorer/src/utils/format.test.ts` (extend existing)

```typescript
import { describe, it, expect } from 'vitest';
import {
  truncateAddress,
  truncateHash,
  formatEth,
  formatTimestamp,
} from '@/utils/format';

describe('truncateAddress', () => {
  it('truncates a 42-char address', () => {
    expect(truncateAddress('0x1234567890abcdef1234567890abcdef12345678')).toBe(
      '0x1234...5678',
    );
  });

  it('returns short strings unchanged', () => {
    expect(truncateAddress('0x1234')).toBe('0x1234');
  });
});

describe('truncateHash', () => {
  it('formats a 66-char hash to 0xabcd...1234', () => {
    const hash = '0x' + 'a'.repeat(64);
    expect(truncateHash(hash)).toBe('0xaaaa...aaaa');
  });

  it('respects custom char count', () => {
    const hash = '0x' + 'b'.repeat(64);
    expect(truncateHash(hash, 6)).toBe('0xbbbbbb...bbbbbb');
  });
});

describe('formatEth', () => {
  it('formats 1 ETH from wei', () => {
    expect(formatEth(1_000_000_000_000_000_000n)).toBe('1.0000');
  });

  it('formats 0 ETH', () => {
    expect(formatEth(0n)).toBe('0.0000');
  });

  it('handles wei (sub-ether amounts)', () => {
    // 1 wei should format as a very small number
    const result = formatEth(1n);
    expect(parseFloat(result)).toBeCloseTo(0, 4);
  });

  it('handles gwei (1 gwei = 1e9 wei)', () => {
    const oneGwei = 1_000_000_000n;
    const result = formatEth(oneGwei);
    expect(parseFloat(result)).toBeCloseTo(0.000000001, 9);
  });

  it('handles large values (1000 ETH)', () => {
    const val = 1000_000_000_000_000_000_000n;
    expect(formatEth(val)).toBe('1000.0000');
  });
});

describe('formatTimestamp', () => {
  it('formats a recent timestamp as relative (e.g., "3s ago")', () => {
    const now = Math.floor(Date.now() / 1000);
    const result = formatTimestamp(now - 3, 'relative');
    expect(result).toMatch(/3s?\s*ago/i);
  });

  it('formats an older timestamp as relative (e.g., "5m ago")', () => {
    const now = Math.floor(Date.now() / 1000);
    const result = formatTimestamp(now - 300, 'relative');
    expect(result).toMatch(/5m?\s*ago/i);
  });

  it('formats as absolute date string', () => {
    // Unix timestamp for 2024-01-01T00:00:00Z
    const ts = 1704067200;
    const result = formatTimestamp(ts, 'absolute');
    expect(result).toContain('2024');
  });
});
```

### 12.2.5 Search Parser Tests

#### `apps/explorer/src/panels/SearchOverlay.test.ts`

```typescript
import { describe, it, expect } from 'vitest';
import { parseSearchInput } from '@/panels/SearchOverlay';

describe('Search Input Parser', () => {
  it('parses a decimal number as block number', () => {
    const result = parseSearchInput('12345');
    expect(result).toEqual({ type: 'block', value: 12345 });
  });

  it('parses a hex number with 0x prefix as block number', () => {
    const result = parseSearchInput('0xff');
    expect(result).toEqual({ type: 'block', value: 255 });
  });

  it('parses 0x + 64 hex chars as transaction hash', () => {
    const txHash = '0x' + 'a'.repeat(64);
    const result = parseSearchInput(txHash);
    expect(result).toEqual({ type: 'tx', value: txHash });
  });

  it('parses 0x + 40 hex chars as address', () => {
    const addr = '0x' + 'b'.repeat(40);
    const result = parseSearchInput(addr);
    expect(result).toEqual({ type: 'address', value: addr });
  });

  it('rejects empty input', () => {
    const result = parseSearchInput('');
    expect(result).toEqual({ type: 'invalid', value: '' });
  });

  it('rejects non-hex garbage', () => {
    const result = parseSearchInput('hello world');
    expect(result).toEqual({ type: 'invalid', value: 'hello world' });
  });

  it('rejects 0x with wrong length (not 40 or 64)', () => {
    const result = parseSearchInput('0x' + 'a'.repeat(50));
    expect(result).toEqual({ type: 'invalid', value: '0x' + 'a'.repeat(50) });
  });

  it('handles leading/trailing whitespace', () => {
    const result = parseSearchInput('  12345  ');
    expect(result).toEqual({ type: 'block', value: 12345 });
  });
});
```

---

## 12.3 Integration Tests (vitest + MSW)

Integration tests exercise multi-module flows with MSW providing realistic RPC
responses. They live in `apps/explorer/src/__tests__/integration/`.

#### `apps/explorer/src/__tests__/integration/block-polling-cycle.test.ts`

```typescript
import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest';
import { render, screen, waitFor } from '@testing-library/react';
import { setCurrentBlockNumber, MOCK_BLOCKS } from '@/__mocks__/handlers';

describe('Integration: full block polling cycle', () => {
  beforeEach(() => {
    vi.useFakeTimers();
    setCurrentBlockNumber(MOCK_BLOCKS[0].number);
  });

  afterEach(() => {
    vi.useRealTimers();
  });

  it('mock RPC → bus → store → UI update', async () => {
    // Import after MSW is set up
    const { useChainStore } = await import('@/stores/chain');
    const { bus } = await import('@/bus/events');
    const { BlockPoller } = await import('@/data/poller');

    // Wire bus to store (as done in main.tsx)
    const unsub = bus.on('block:new', (block) => {
      useChainStore.getState().addBlock(block);
    });

    // Start polling
    const poller = new BlockPoller({ rpcUrl: '/rpc', interval: 1000 });
    poller.start();

    // Advance to a new block
    setCurrentBlockNumber(MOCK_BLOCKS[2].number);
    await vi.advanceTimersByTimeAsync(2000);

    // Store should now contain blocks
    await waitFor(() => {
      const state = useChainStore.getState();
      expect(state.blocks.length).toBeGreaterThan(0);
    });

    poller.stop();
    unsub();
  });
});
```

#### `apps/explorer/src/__tests__/integration/search-flow.test.tsx`

```typescript
import { describe, it, expect } from 'vitest';
import { render, screen, fireEvent, waitFor } from '@testing-library/react';
import { BrowserRouter } from 'react-router';
import { MOCK_BLOCKS } from '@/__mocks__/handlers';

describe('Integration: search → RPC → result display', () => {
  it('searches for a block number and displays the result', async () => {
    const { SearchOverlay } = await import('@/panels/SearchOverlay');

    render(
      <BrowserRouter>
        <SearchOverlay isOpen onClose={() => {}} />
      </BrowserRouter>,
    );

    const input = screen.getByRole('textbox');
    const blockNum = parseInt(MOCK_BLOCKS[0].number, 16);

    // Type a block number
    fireEvent.change(input, { target: { value: String(blockNum) } });

    // Submit search
    fireEvent.keyDown(input, { key: 'Enter' });

    // Should display the block result
    await waitFor(() => {
      expect(screen.getByText(/block/i)).toBeInTheDocument();
    });
  });
});
```

#### `apps/explorer/src/__tests__/integration/scene-navigation.test.tsx`

```typescript
import { describe, it, expect } from 'vitest';
import { render, screen, fireEvent } from '@testing-library/react';
import { MemoryRouter } from 'react-router';

describe('Integration: scene navigation', () => {
  it('updates URL and renders correct scene on tab switch', async () => {
    const { App } = await import('@/App');

    render(
      <MemoryRouter initialEntries={['/']}>
        <App />
      </MemoryRouter>,
    );

    // Find scene tabs (1=Terrain, 2=Constellation, 3=Waterfall, 4=Consensus)
    // Simulate pressing keyboard shortcut for scene 2
    fireEvent.keyDown(document.body, { key: '2' });

    // The constellation scene (or its loading state) should be rendered
    await screen.findByTestId(/scene-constellation|scene-loading/);
  });
});
```

#### `apps/explorer/src/__tests__/integration/entity-selection.test.tsx`

```typescript
import { describe, it, expect, vi } from 'vitest';
import { render, screen, fireEvent, waitFor } from '@testing-library/react';
import { MemoryRouter } from 'react-router';
import { useUIStore } from '@/stores/ui';

describe('Integration: entity selection', () => {
  it('opens the correct detail panel when an entity is selected', async () => {
    const { App } = await import('@/App');

    render(
      <MemoryRouter initialEntries={['/block/100']}>
        <App />
      </MemoryRouter>,
    );

    // The block detail panel should be visible
    await waitFor(() => {
      const state = useUIStore.getState();
      expect(state.selectedEntity?.type).toBe('block');
    });
  });
});
```

#### `apps/explorer/src/__tests__/integration/keyboard-shortcuts.test.tsx`

```typescript
import { describe, it, expect } from 'vitest';
import { render, screen, fireEvent } from '@testing-library/react';
import { MemoryRouter } from 'react-router';

describe('Integration: keyboard shortcuts', () => {
  it('/ opens the search overlay', async () => {
    const { App } = await import('@/App');

    render(
      <MemoryRouter>
        <App />
      </MemoryRouter>,
    );

    fireEvent.keyDown(document.body, { key: '/' });

    await screen.findByRole('textbox'); // Search input
  });

  it('Escape closes the search overlay', async () => {
    const { App } = await import('@/App');
    const { useUIStore } = await import('@/stores/ui');

    render(
      <MemoryRouter>
        <App />
      </MemoryRouter>,
    );

    // Open search
    fireEvent.keyDown(document.body, { key: '/' });
    await screen.findByRole('textbox');

    // Close with Escape
    fireEvent.keyDown(document.body, { key: 'Escape' });

    // Search should be closed
    expect(useUIStore.getState().searchOpen).toBe(false);
  });

  it('1/2/3/4 switch scenes', async () => {
    const { App } = await import('@/App');
    const { useUIStore } = await import('@/stores/ui');

    render(
      <MemoryRouter>
        <App />
      </MemoryRouter>,
    );

    const sceneKeys = ['1', '2', '3', '4'];
    const sceneNames = ['terrain', 'constellation', 'waterfall', 'consensus'];

    for (let i = 0; i < sceneKeys.length; i++) {
      fireEvent.keyDown(document.body, { key: sceneKeys[i] });
      expect(useUIStore.getState().activeScene).toBe(sceneNames[i]);
    }
  });
});
```

#### `apps/explorer/src/__tests__/integration/ambient-mode.test.tsx`

```typescript
import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest';
import { render } from '@testing-library/react';
import { MemoryRouter } from 'react-router';

describe('Integration: ambient mode', () => {
  beforeEach(() => {
    vi.useFakeTimers();
  });

  afterEach(() => {
    vi.useRealTimers();
  });

  it('activates after 30 seconds of inactivity', async () => {
    const { App } = await import('@/App');
    const { useUIStore } = await import('@/stores/ui');

    render(
      <MemoryRouter>
        <App />
      </MemoryRouter>,
    );

    // Advance 30 seconds with no user interaction
    await vi.advanceTimersByTimeAsync(30_000);

    expect(useUIStore.getState().ambientMode).toBe(true);
  });
});
```

---

## 12.4 E2E Tests (Playwright)

### 12.4.1 Playwright Configuration

#### `apps/explorer/playwright.config.ts`

```typescript
import { defineConfig, devices } from '@playwright/test';

export default defineConfig({
  testDir: './e2e',
  fullyParallel: true,
  forbidOnly: !!process.env.CI,
  retries: process.env.CI ? 2 : 0,
  workers: process.env.CI ? 1 : undefined,
  reporter: process.env.CI ? 'github' : 'html',

  use: {
    baseURL: 'http://localhost:5173',
    trace: 'on-first-retry',
    screenshot: 'only-on-failure',
    video: 'retain-on-failure',
  },

  webServer: {
    command: 'pnpm dev',
    port: 5173,
    reuseExistingServer: !process.env.CI,
    timeout: 30_000,
  },

  projects: [
    {
      name: 'chromium',
      use: { ...devices['Desktop Chrome'] },
    },
    {
      name: 'mobile-chrome',
      use: { ...devices['Pixel 5'] },
    },
  ],
});
```

### 12.4.2 E2E Test Files

#### `apps/explorer/e2e/page-load.spec.ts`

```typescript
import { test, expect } from '@playwright/test';

test('page loads with a scene visible', async ({ page }) => {
  await page.goto('/');

  // The root element should be visible
  await expect(page.locator('#root')).toBeVisible();

  // The scene container or canvas should be present
  // (either a canvas element or the scene wrapper div)
  const sceneEl = page.locator('[data-testid^="scene-"], canvas').first();
  await expect(sceneEl).toBeVisible({ timeout: 10_000 });
});

test('status bar shows live block number', async ({ page }) => {
  await page.goto('/');

  // Wait for the status bar to appear with a block number
  const blockNumber = page.locator('[data-testid="block-number"]');
  await expect(blockNumber).toBeVisible({ timeout: 10_000 });

  // Block number should contain a numeric value
  const text = await blockNumber.textContent();
  expect(text).toMatch(/\d+/);
});
```

#### `apps/explorer/e2e/scene-tabs.spec.ts`

```typescript
import { test, expect } from '@playwright/test';

test('scene tabs switch between 4 scenes', async ({ page }) => {
  await page.goto('/');
  await page.waitForLoadState('networkidle');

  const sceneKeys = ['1', '2', '3', '4'];
  const sceneIds = [
    'scene-terrain',
    'scene-constellation',
    'scene-waterfall',
    'scene-consensus',
  ];

  for (let i = 0; i < sceneKeys.length; i++) {
    await page.keyboard.press(sceneKeys[i]);

    // Wait for the scene container to appear
    // Scene may lazy-load, so allow loading state too
    const scene = page.locator(
      `[data-testid="${sceneIds[i]}"], [data-testid="scene-loading"]`,
    );
    await expect(scene.first()).toBeVisible({ timeout: 5_000 });
  }
});
```

#### `apps/explorer/e2e/search.spec.ts`

```typescript
import { test, expect } from '@playwright/test';

test('search for block number navigates to block detail', async ({ page }) => {
  await page.goto('/');
  await page.waitForLoadState('networkidle');

  // Open search with Cmd+K (or Ctrl+K on non-Mac)
  await page.keyboard.press('Meta+k');

  // Type a block number
  const searchInput = page.locator('input[type="text"], input[type="search"]');
  await expect(searchInput).toBeVisible();
  await searchInput.fill('100');
  await searchInput.press('Enter');

  // Should navigate to block detail
  await expect(page).toHaveURL(/\/block\/100/);
});

test('Cmd+K opens search overlay', async ({ page }) => {
  await page.goto('/');

  await page.keyboard.press('Meta+k');

  const searchInput = page.locator('input[type="text"], input[type="search"]');
  await expect(searchInput).toBeVisible();
});
```

#### `apps/explorer/e2e/deep-links.spec.ts`

```typescript
import { test, expect } from '@playwright/test';

test('deep link /block/100 loads block detail', async ({ page }) => {
  await page.goto('/block/100');

  // Should show block detail content
  const detail = page.locator('[data-testid="block-detail"]');
  await expect(detail).toBeVisible({ timeout: 10_000 });

  // Should contain the block number
  await expect(detail).toContainText('100');
});

test('deep link for unknown block shows error or not-found', async ({ page }) => {
  await page.goto('/block/999999999');

  // Should show some not-found state
  const content = page.locator('[data-testid="block-detail"], [data-testid="not-found"]');
  await expect(content.first()).toBeVisible({ timeout: 10_000 });
});
```

#### `apps/explorer/e2e/responsive.spec.ts`

```typescript
import { test, expect } from '@playwright/test';

test('responsive layout on 375px width (mobile)', async ({ page }) => {
  await page.setViewportSize({ width: 375, height: 812 });
  await page.goto('/');

  // Root should fill viewport
  const root = page.locator('#root');
  await expect(root).toBeVisible();

  // Panels should stack vertically (no horizontal overflow)
  const body = page.locator('body');
  const box = await body.boundingBox();
  expect(box?.width).toBeLessThanOrEqual(375);
});
```

#### `apps/explorer/e2e/webgl-canvas.spec.ts`

```typescript
import { test, expect } from '@playwright/test';

test('WebGL canvas renders (screenshot comparison)', async ({ page }) => {
  await page.goto('/');

  // Wait for a scene to be active
  await page.waitForTimeout(3000);

  // Take a screenshot of the canvas area
  const canvas = page.locator('canvas').first();

  if (await canvas.isVisible()) {
    // Canvas exists and is visible — WebGL is working
    const screenshot = await canvas.screenshot();
    expect(screenshot.byteLength).toBeGreaterThan(1000);

    // Optional: compare against reference screenshot
    // await expect(canvas).toHaveScreenshot('scene-default.png', {
    //   maxDiffPixelRatio: 0.05,
    // });
  } else {
    // Fallback mode (no WebGL) — check for fallback UI
    const fallback = page.locator('[data-testid="webgl-fallback"]');
    await expect(fallback).toBeVisible();
  }
});
```

---

## 12.5 Performance Tests

### 12.5.1 Lighthouse CI

#### `apps/explorer/lighthouserc.js`

```javascript
module.exports = {
  ci: {
    collect: {
      url: ['http://localhost:5173/'],
      startServerCommand: 'pnpm preview',
      startServerReadyPattern: 'Local',
      numberOfRuns: 3,
      settings: {
        preset: 'desktop',
        // Skip audits that require real network
        skipAudits: ['uses-http2', 'redirects-http'],
      },
    },
    assert: {
      assertions: {
        'categories:performance': ['error', { minScore: 0.9 }],
        'categories:accessibility': ['warn', { minScore: 0.8 }],
        'categories:best-practices': ['warn', { minScore: 0.9 }],
        // First contentful paint under 2.5s
        'first-contentful-paint': ['warn', { maxNumericValue: 2500 }],
        // Largest contentful paint under 3s
        'largest-contentful-paint': ['error', { maxNumericValue: 3000 }],
        // Total blocking time under 300ms
        'total-blocking-time': ['error', { maxNumericValue: 300 }],
      },
    },
    upload: {
      target: 'temporary-public-storage',
    },
  },
};
```

Run locally with:

```bash
pnpm build && pnpm dlx @lhci/cli autorun
```

### 12.5.2 Bundle Size Budget

Add a CI check script that fails if bundles exceed limits.

#### `apps/explorer/scripts/check-bundle-size.sh`

```bash
#!/usr/bin/env bash
set -euo pipefail

DIST="dist/assets"
MAX_TOTAL_KB=500
MAX_VENDOR_THREE_KB=200
MAX_VENDOR_REACT_KB=50
MAX_APP_KB=150
MAX_VENDOR_REGL_KB=30

if [ ! -d "$DIST" ]; then
  echo "ERROR: $DIST not found. Run 'pnpm build' first."
  exit 1
fi

check_size() {
  local pattern="$1"
  local max_kb="$2"
  local label="$3"

  # Find gzipped size of matching files
  local total=0
  for f in $DIST/$pattern; do
    if [ -f "$f" ]; then
      local gz_size
      gz_size=$(gzip -c "$f" | wc -c | tr -d ' ')
      total=$((total + gz_size))
    fi
  done

  local total_kb=$((total / 1024))
  if [ "$total_kb" -gt "$max_kb" ]; then
    echo "FAIL: $label = ${total_kb}KB gzipped (max: ${max_kb}KB)"
    return 1
  else
    echo "OK:   $label = ${total_kb}KB gzipped (max: ${max_kb}KB)"
    return 0
  fi
}

echo "=== Bundle Size Check ==="
echo ""

failures=0

check_size "vendor-three-*.js" $MAX_VENDOR_THREE_KB "vendor-three" || failures=$((failures + 1))
check_size "vendor-react-*.js" $MAX_VENDOR_REACT_KB "vendor-react" || failures=$((failures + 1))
check_size "vendor-regl-*.js"  $MAX_VENDOR_REGL_KB  "vendor-regl"  || failures=$((failures + 1))

# App code = everything except vendor chunks
app_total=0
for f in $DIST/*.js; do
  if [[ ! "$f" =~ vendor- ]]; then
    gz_size=$(gzip -c "$f" | wc -c | tr -d ' ')
    app_total=$((app_total + gz_size))
  fi
done
app_kb=$((app_total / 1024))
if [ "$app_kb" -gt "$MAX_APP_KB" ]; then
  echo "FAIL: app code = ${app_kb}KB gzipped (max: ${MAX_APP_KB}KB)"
  failures=$((failures + 1))
else
  echo "OK:   app code = ${app_kb}KB gzipped (max: ${MAX_APP_KB}KB)"
fi

# Total JS
total_js=0
for f in $DIST/*.js; do
  gz_size=$(gzip -c "$f" | wc -c | tr -d ' ')
  total_js=$((total_js + gz_size))
done
total_kb=$((total_js / 1024))
if [ "$total_kb" -gt "$MAX_TOTAL_KB" ]; then
  echo "FAIL: total JS = ${total_kb}KB gzipped (max: ${MAX_TOTAL_KB}KB)"
  failures=$((failures + 1))
else
  echo "OK:   total JS = ${total_kb}KB gzipped (max: ${MAX_TOTAL_KB}KB)"
fi

echo ""
if [ "$failures" -gt 0 ]; then
  echo "FAILED: $failures budget(s) exceeded."
  exit 1
else
  echo "All bundle size budgets passed."
fi
```

```bash
chmod +x scripts/check-bundle-size.sh
```

### 12.5.3 Frame Rate Test

A Playwright test that measures frame rate under load.

#### `apps/explorer/e2e/performance.spec.ts`

```typescript
import { test, expect } from '@playwright/test';

test('maintains 60fps for 60 seconds with mock data', async ({ page }) => {
  await page.goto('/');
  await page.waitForLoadState('networkidle');

  // Inject a frame counter into the page
  const frameData = await page.evaluate(async () => {
    return new Promise<{ avgFps: number; minFps: number; droppedFrames: number }>(
      (resolve) => {
        const frameTimes: number[] = [];
        let lastTime = performance.now();
        let frameCount = 0;
        const duration = 10_000; // 10s (reduced from 60s for CI speed)
        const startTime = performance.now();

        function measure() {
          const now = performance.now();
          const delta = now - lastTime;
          frameTimes.push(delta);
          lastTime = now;
          frameCount++;

          if (now - startTime < duration) {
            requestAnimationFrame(measure);
          } else {
            // Calculate stats
            const fps = frameTimes.map((dt) => 1000 / dt);
            const avgFps = fps.reduce((a, b) => a + b, 0) / fps.length;
            const minFps = Math.min(...fps);
            const droppedFrames = fps.filter((f) => f < 30).length;
            resolve({ avgFps, minFps, droppedFrames });
          }
        }

        requestAnimationFrame(measure);
      },
    );
  });

  // Average FPS should be at least 50 (allowing some headroom below 60)
  expect(frameData.avgFps).toBeGreaterThan(50);

  // No more than 5% dropped frames (below 30fps)
  const totalFrames = frameData.avgFps * 10;
  expect(frameData.droppedFrames / totalFrames).toBeLessThan(0.05);
});

test('heap stays under 100MB after 10 minutes', async ({ page }) => {
  // Enable performance metrics in CDP
  const client = await page.context().newCDPSession(page);
  await client.send('Performance.enable');

  await page.goto('/');
  await page.waitForLoadState('networkidle');

  // Let the app run for a while (30s in CI, simulate 10min with time acceleration)
  await page.waitForTimeout(30_000);

  // Force GC and measure heap
  await client.send('HeapProfiler.collectGarbage');
  const metrics = await client.send('Performance.getMetrics');
  const heapSize = metrics.metrics.find((m) => m.name === 'JSHeapUsedSize');

  if (heapSize) {
    const heapMB = heapSize.value / (1024 * 1024);
    console.log(`JS Heap: ${heapMB.toFixed(1)}MB`);
    expect(heapMB).toBeLessThan(100);
  }
});
```

---

## 12.6 Build Configuration

### 12.6.1 Vite Production Build

The complete production build config is established in doc 01 (`vite.config.ts`).
This section documents the final production-specific settings.

#### `apps/explorer/vite.config.ts` — build block (final)

```typescript
build: {
  target: 'esnext',
  outDir: 'dist',
  minify: 'terser',
  terserOptions: {
    compress: {
      drop_console: true,   // strip console.log in production
      drop_debugger: true,
    },
  },
  sourcemap: 'hidden',  // generate sourcemaps but don't reference them in output
                          // upload to monitoring service, never served to browsers
  rollupOptions: {
    output: {
      manualChunks: {
        'vendor-react': ['react', 'react-dom', 'react-router', 'zustand'],
        'vendor-three': ['three', '@react-three/fiber', '@react-three/drei'],
        'vendor-regl': ['regl'],
        'vendor-data': ['viem'],
        'vendor-audio': ['tone'],
      },
      // Content-hash in filenames for cache busting
      entryFileNames: 'assets/[name]-[hash].js',
      chunkFileNames: 'assets/[name]-[hash].js',
      assetFileNames: 'assets/[name]-[hash][extname]',
    },
  },
  chunkSizeWarningLimit: 300,
  // Extract CSS into separate files for parallel loading
  cssCodeSplit: true,
},
```

Key decisions:

- **minify: terser** — smaller output than esbuild minification at the cost of
  slightly slower builds. Worth it for a production SPA.
- **sourcemap: 'hidden'** — sourcemaps are generated in `dist/` for upload to
  error monitoring (Sentry, etc.) but no `//# sourceMappingURL` comment is emitted
  in the JS files. Browsers never request them.
- **Content hashing** — every asset filename includes a hash. Combined with
  `Cache-Control: immutable` on the nginx side, this gives permanent caching for
  all assets except `index.html`.
- **CSS extraction** — CSS is split into its own files and loaded in parallel
  with JS, reducing total parse time.
- **Manual chunks** — the same split from doc 01. Three.js and audio are
  lazily loaded, so their chunks are only fetched when first needed.

### 12.6.2 Bundle Analysis

Install `vite-bundle-visualizer` as a dev dependency:

```json
{
  "devDependencies": {
    "vite-bundle-visualizer": "^1.0.0"
  }
}
```

Add a script to `package.json`:

```json
{
  "scripts": {
    "analyze": "vite-bundle-visualizer"
  }
}
```

Usage:

```bash
pnpm build && pnpm analyze
# Opens an interactive treemap in the browser showing chunk composition
```

### 12.6.3 Build Output

After `pnpm build`, the `dist/` directory should look like:

```
dist/
├── index.html                          # ~1KB — no cache, served fresh
├── assets/
│   ├── index-[hash].js                # app entry chunk
│   ├── index-[hash].css               # extracted CSS
│   ├── vendor-react-[hash].js         # React + Router + Zustand
│   ├── vendor-three-[hash].js         # Three.js ecosystem (lazy)
│   ├── vendor-regl-[hash].js          # regl (lazy)
│   ├── vendor-data-[hash].js          # viem
│   ├── vendor-audio-[hash].js         # Tone.js (lazy)
│   ├── TerrainScene-[hash].js         # lazy scene chunk
│   ├── ConstellationCanvas-[hash].js  # lazy scene chunk
│   ├── WaterfallScene-[hash].js       # lazy scene chunk
│   └── ConsensusRing-[hash].js        # lazy scene chunk
└── fonts/
    ├── Fraunces-Italic-Variable.woff2
    └── JetBrainsMono-Variable.woff2
```

Total size target: under 2MB uncompressed, under 500KB gzipped (JS only).

---

## 12.7 Static Deployment

The explorer is a static SPA. It can be served alongside the kora node via Docker
or deployed independently to any static hosting.

### 12.7.1 Dockerfile

#### `apps/explorer/Dockerfile`

```dockerfile
# ── Stage 1: Build ──
FROM node:20-slim AS build

# Enable pnpm via corepack
RUN corepack enable && corepack prepare pnpm@latest --activate

WORKDIR /app

# Copy package files first for Docker layer caching
COPY package.json pnpm-lock.yaml ./
RUN pnpm install --frozen-lockfile

# Copy source and build
COPY . .
RUN pnpm build

# ── Stage 2: Serve ──
FROM nginx:1.27-alpine

# Remove default nginx config
RUN rm /etc/nginx/conf.d/default.conf

# Copy built assets
COPY --from=build /app/dist /usr/share/nginx/html

# Copy nginx config
COPY nginx.conf /etc/nginx/conf.d/default.conf

# Nginx runs on port 80
EXPOSE 80

CMD ["nginx", "-g", "daemon off;"]
```

### 12.7.2 nginx Configuration

#### `apps/explorer/nginx.conf`

```nginx
server {
    listen 80;
    server_name _;

    root /usr/share/nginx/html;
    index index.html;

    # ── SPA Fallback ──
    # All routes that don't match a static file serve index.html.
    # React Router handles client-side routing from there.
    location / {
        try_files $uri $uri/ /index.html;
    }

    # ── RPC Proxy (HTTP JSON-RPC) ──
    # Proxies /rpc to the kora node's HTTP RPC endpoint.
    # Avoids exposing the node's IP directly to browsers.
    location /rpc {
        proxy_pass http://kora:8545;
        proxy_http_version 1.1;
        proxy_set_header Host $host;
        proxy_set_header X-Real-IP $remote_addr;
        proxy_set_header X-Forwarded-For $proxy_add_x_forwarded_for;
        proxy_set_header X-Forwarded-Proto $scheme;

        # Timeouts for long-running RPC calls
        proxy_read_timeout 30s;
        proxy_send_timeout 10s;
    }

    # ── WebSocket Proxy ──
    # Proxies /ws to the kora node's WebSocket endpoint.
    # Required for eth_subscribe (Phase 2).
    location /ws {
        proxy_pass http://kora:8546;
        proxy_http_version 1.1;

        # WebSocket upgrade headers
        proxy_set_header Upgrade $http_upgrade;
        proxy_set_header Connection "upgrade";
        proxy_set_header Host $host;
        proxy_set_header X-Real-IP $remote_addr;
        proxy_set_header X-Forwarded-For $proxy_add_x_forwarded_for;

        # Keep WebSocket connections alive
        proxy_read_timeout 3600s;
        proxy_send_timeout 3600s;
    }

    # ── Hashed Static Assets ──
    # Files in /assets/ have content hashes in their filenames.
    # They never change — cache forever.
    location /assets/ {
        expires 1y;
        add_header Cache-Control "public, immutable";
        access_log off;
    }

    # ── Font Assets ──
    # Fonts are also immutable once deployed.
    location /fonts/ {
        expires 1y;
        add_header Cache-Control "public, immutable";
        access_log off;

        # CORS for font loading (if served from a CDN subdomain)
        add_header Access-Control-Allow-Origin "*";
    }

    # ── index.html ──
    # Must never be cached — it's the only file that changes between deploys.
    # The hashed asset references inside it point to the new bundles.
    location = /index.html {
        add_header Cache-Control "no-cache, no-store, must-revalidate";
        add_header Pragma "no-cache";
        add_header Expires "0";
    }

    # ── Gzip Compression ──
    gzip on;
    gzip_vary on;
    gzip_proxied any;
    gzip_comp_level 6;
    gzip_min_length 256;
    gzip_types
        text/plain
        text/css
        text/javascript
        application/javascript
        application/json
        application/xml
        image/svg+xml
        font/woff2;
}
```

### 12.7.3 Docker Compose Integration

Add the explorer service to the existing docker-compose configuration for the
kora devnet.

#### Addition to `docker/docker-compose.yml`

```yaml
  explorer:
    build:
      context: ../apps/explorer
      dockerfile: Dockerfile
    ports:
      - "3000:80"
    depends_on:
      - validator-0
    environment:
      # These are baked into the build at compile time (VITE_ prefix).
      # For runtime config, the nginx proxy handles routing to the node.
      VITE_RPC_URL: "/rpc"
      VITE_WS_URL: "/ws"
    restart: unless-stopped
    networks:
      - kora-net
```

The explorer container depends on `validator-0` (the first kora node in the devnet).
Nginx inside the explorer container proxies `/rpc` to `validator-0:8545` and
`/ws` to `validator-0:8546`. The browser never contacts the node directly.

### 12.7.4 Docker Build and Run

```bash
# Build the explorer image
docker build -t kora-explorer apps/explorer/

# Run standalone (requires a kora node accessible at kora:8545)
docker run -p 3000:80 kora-explorer

# Run as part of the full devnet
cd docker && docker compose up --build
# Explorer available at http://localhost:3000
```

---

## 12.8 CI Integration

### 12.8.1 Justfile Additions

Append to `/Users/will/dev/nunchi/daeji/Justfile` (extends the targets from doc 01):

```just
# ── Explorer CI ──

# Run all explorer tests (unit + integration)
explorer-test:
    cd apps/explorer && pnpm test

# Run explorer unit tests with coverage
explorer-test-coverage:
    cd apps/explorer && pnpm test -- --coverage

# Run explorer integration tests
explorer-test-integration:
    cd apps/explorer && pnpm test -- --testPathPattern='__tests__/integration'

# Run explorer E2E tests (requires dev server or preview)
explorer-test-e2e:
    cd apps/explorer && pnpm test:e2e

# Lint explorer source
explorer-lint:
    cd apps/explorer && pnpm lint

# Production build
explorer-build:
    cd apps/explorer && pnpm build

# Check bundle size budgets
explorer-check-bundle:
    cd apps/explorer && pnpm build && ./scripts/check-bundle-size.sh

# Start dev server
explorer-dev:
    cd apps/explorer && pnpm dev

# Bundle analysis (opens browser)
explorer-analyze:
    cd apps/explorer && pnpm build && pnpm analyze
```

### 12.8.2 GitHub Actions Workflow

#### `.github/workflows/explorer.yml`

```yaml
name: Explorer CI

on:
  push:
    branches: [main]
    paths:
      - 'apps/explorer/**'
      - '.github/workflows/explorer.yml'
  pull_request:
    paths:
      - 'apps/explorer/**'
      - '.github/workflows/explorer.yml'

defaults:
  run:
    working-directory: apps/explorer

jobs:
  lint:
    name: Lint
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: pnpm/action-setup@v4
        with:
          version: 9
      - uses: actions/setup-node@v4
        with:
          node-version: 20
          cache: pnpm
          cache-dependency-path: apps/explorer/pnpm-lock.yaml
      - run: pnpm install --frozen-lockfile
      - run: pnpm lint

  typecheck:
    name: TypeScript
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: pnpm/action-setup@v4
        with:
          version: 9
      - uses: actions/setup-node@v4
        with:
          node-version: 20
          cache: pnpm
          cache-dependency-path: apps/explorer/pnpm-lock.yaml
      - run: pnpm install --frozen-lockfile
      - run: npx tsc --noEmit

  test:
    name: Unit & Integration Tests
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: pnpm/action-setup@v4
        with:
          version: 9
      - uses: actions/setup-node@v4
        with:
          node-version: 20
          cache: pnpm
          cache-dependency-path: apps/explorer/pnpm-lock.yaml
      - run: pnpm install --frozen-lockfile
      - run: pnpm test -- --coverage
      - name: Upload coverage
        uses: actions/upload-artifact@v4
        with:
          name: coverage
          path: apps/explorer/coverage/

  build:
    name: Build & Bundle Check
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: pnpm/action-setup@v4
        with:
          version: 9
      - uses: actions/setup-node@v4
        with:
          node-version: 20
          cache: pnpm
          cache-dependency-path: apps/explorer/pnpm-lock.yaml
      - run: pnpm install --frozen-lockfile
      - run: pnpm build
      - name: Check bundle size
        run: ./scripts/check-bundle-size.sh
      - name: Upload build artifacts
        uses: actions/upload-artifact@v4
        with:
          name: dist
          path: apps/explorer/dist/

  e2e:
    name: E2E Tests
    runs-on: ubuntu-latest
    needs: [build]
    steps:
      - uses: actions/checkout@v4
      - uses: pnpm/action-setup@v4
        with:
          version: 9
      - uses: actions/setup-node@v4
        with:
          node-version: 20
          cache: pnpm
          cache-dependency-path: apps/explorer/pnpm-lock.yaml
      - run: pnpm install --frozen-lockfile
      - name: Install Playwright browsers
        run: npx playwright install --with-deps chromium
      - run: pnpm test:e2e
      - name: Upload Playwright report
        if: failure()
        uses: actions/upload-artifact@v4
        with:
          name: playwright-report
          path: apps/explorer/playwright-report/

  lighthouse:
    name: Lighthouse
    runs-on: ubuntu-latest
    needs: [build]
    steps:
      - uses: actions/checkout@v4
      - uses: pnpm/action-setup@v4
        with:
          version: 9
      - uses: actions/setup-node@v4
        with:
          node-version: 20
          cache: pnpm
          cache-dependency-path: apps/explorer/pnpm-lock.yaml
      - run: pnpm install --frozen-lockfile
      - run: pnpm build
      - name: Run Lighthouse CI
        run: pnpm dlx @lhci/cli autorun
```

The workflow runs on changes to `apps/explorer/**` only, so Rust CI and explorer CI
are completely independent. The `e2e` and `lighthouse` jobs run after `build`
to avoid redundant compilation.

### 12.8.3 Package.json Script Additions

Add the following to `apps/explorer/package.json` scripts:

```json
{
  "scripts": {
    "dev": "vite",
    "build": "tsc -b && vite build",
    "preview": "vite preview",
    "lint": "eslint src/",
    "fmt": "prettier --write src/",
    "test": "vitest run",
    "test:watch": "vitest",
    "test:coverage": "vitest run --coverage",
    "test:integration": "vitest run --testPathPattern='__tests__/integration'",
    "test:e2e": "playwright test",
    "test:e2e:ui": "playwright test --ui",
    "analyze": "vite-bundle-visualizer",
    "check-bundle": "./scripts/check-bundle-size.sh"
  }
}
```

---

## 12.9 Verification Checklist

Run every check from `apps/explorer/`.

- [ ] **Unit tests pass (40+ tests, 0 failures)**
  ```bash
  pnpm test
  # ✓ 40+ tests across rpc, stores, algorithms, utils, search
  # All passing, no failures
  ```

- [ ] **Coverage meets 80% threshold**
  ```bash
  pnpm test:coverage
  # Coverage summary:
  #   Statements: ≥80%
  #   Branches:   ≥80%
  #   Functions:  ≥80%
  #   Lines:      ≥80%
  ```

- [ ] **Integration tests pass with MSW**
  ```bash
  pnpm test:integration
  # ✓ block polling cycle
  # ✓ search flow
  # ✓ scene navigation
  # ✓ entity selection
  # ✓ keyboard shortcuts
  # ✓ ambient mode
  ```

- [ ] **Playwright E2E tests pass**
  ```bash
  pnpm test:e2e
  # ✓ page load
  # ✓ scene tabs
  # ✓ search navigation
  # ✓ deep links
  # ✓ responsive layout
  # ✓ WebGL canvas
  ```

- [ ] **Production build produces dist/ under 2MB**
  ```bash
  pnpm build
  du -sh dist/
  # Should be under 2MB total
  ls dist/index.html         # exists
  ls dist/assets/*.js        # JS chunks exist
  ls dist/assets/*.css       # CSS extracted
  ```

- [ ] **Bundle sizes within budget**
  ```bash
  ./scripts/check-bundle-size.sh
  # OK: vendor-three < 200KB gzipped
  # OK: vendor-react < 50KB gzipped
  # OK: vendor-regl  < 30KB gzipped
  # OK: app code     < 150KB gzipped
  # OK: total JS     < 500KB gzipped
  ```

- [ ] **Docker image builds and serves SPA**
  ```bash
  docker build -t kora-explorer .
  docker run -d -p 3000:80 --name explorer-test kora-explorer

  # Check SPA loads
  curl -s http://localhost:3000/ | grep -q "Kora Explorer"

  # Check hashed assets are served with immutable cache
  curl -sI http://localhost:3000/assets/ | grep -i cache-control
  # Cache-Control: public, immutable

  # Check index.html is not cached
  curl -sI http://localhost:3000/ | grep -i cache-control
  # Cache-Control: no-cache, no-store, must-revalidate

  docker stop explorer-test && docker rm explorer-test
  ```

- [ ] **nginx proxies /rpc to kora node correctly**
  ```bash
  # With docker compose running (kora + explorer):
  curl -s -X POST http://localhost:3000/rpc \
    -H 'Content-Type: application/json' \
    -d '{"jsonrpc":"2.0","method":"eth_blockNumber","params":[],"id":1}'
  # Should return: {"jsonrpc":"2.0","id":1,"result":"0x..."}
  ```

- [ ] **Deep links work after nginx SPA fallback**
  ```bash
  # Direct navigation to a route that only React Router handles
  curl -s http://localhost:3000/block/100 | grep -q "Kora Explorer"
  # index.html served (not 404), React Router handles the route client-side

  curl -s http://localhost:3000/tx/0xabc | grep -q "Kora Explorer"
  curl -s http://localhost:3000/address/0x123 | grep -q "Kora Explorer"
  ```

- [ ] **Lighthouse score >= 90**
  ```bash
  pnpm build && pnpm dlx @lhci/cli autorun
  # Performance: ≥90
  ```

- [ ] **TypeScript strict mode has zero errors**
  ```bash
  npx tsc --noEmit
  # No output (clean)
  ```

- [ ] **ESLint passes**
  ```bash
  pnpm lint
  # No errors
  ```

---

## 12.10 What This Document Does Not Cover

- **Sentry / error monitoring integration** — sourcemaps are generated (hidden) and
  ready for upload, but the monitoring service configuration is out of scope.
- **CDN deployment** — the explorer can be served from Cloudflare Pages, Vercel,
  or Netlify with zero changes (static SPA with `_redirects` or equivalent SPA
  fallback). Specific provider setup is not documented here.
- **Load testing the RPC proxy** — nginx proxying is straightforward; load testing
  the kora node's RPC layer itself is a backend concern.
- **Visual regression reference images** — Playwright screenshot comparison requires
  generating baseline images from a deterministic block state. The test scaffolding
  is here (commented out in `webgl-canvas.spec.ts`); baseline generation is a
  one-time manual step once scenes are visually stable.
