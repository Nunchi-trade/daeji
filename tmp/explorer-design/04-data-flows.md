# Data Flows — Chain Data → Visual Elements

How chain data enters the system, transforms, and drives visuals.

---

## Architecture

```
                    ┌─────────────────────────┐
                    │    kora RPC / WS         │
                    │    65.109.61.210:8545    │
                    └──────────┬──────────────┘
                               │
                    ┌──────────▼──────────────┐
                    │    DataLayer             │
                    │                          │
                    │  ┌─ Poller ────────────┐ │    ← Phase 1: polling
                    │  │ eth_blockNumber 1/s │ │
                    │  │ eth_feeHistory  5/s │ │
                    │  └────────────────────┘ │
                    │                          │
                    │  ┌─ Subscriber ────────┐ │    ← Phase 2: WS push
                    │  │ newHeads            │ │
                    │  │ logs               │ │
                    │  │ pendingTransactions │ │
                    │  └────────────────────┘ │
                    │                          │
                    │  ┌─ Cache ─────────────┐ │
                    │  │ blockMap (LRU 500)  │ │
                    │  │ addressSet          │ │
                    │  │ feeHistory (ring)   │ │
                    │  └────────────────────┘ │
                    └──────────┬──────────────┘
                               │
                    ┌──────────▼──────────────┐
                    │    EventBus              │
                    │                          │
                    │  "block:new"    → Block  │
                    │  "tx:confirmed" → Tx     │
                    │  "tx:pending"   → Tx     │
                    │  "fee:update"   → Fee[]  │
                    │  "address:seen" → Addr   │
                    │  "log:new"      → Log    │
                    └──────────┬──────────────┘
                               │
              ┌────────────────┼────────────────┐
              │                │                │
     ┌────────▼───────┐ ┌─────▼──────┐ ┌───────▼────────┐
     │  SceneManager  │ │ PanelState │ │ AudioEngine    │
     │                │ │            │ │ (optional)     │
     │  terrain       │ │ block info │ │ tick on block  │
     │  constellation │ │ gas chart  │ │ tone on tx     │
     │  waterfall     │ │ status     │ │ ambient drone  │
     │  consensus     │ │ search     │ │                │
     └────────────────┘ └────────────┘ └────────────────┘
```

---

## Phase 1: Polling (Works Now)

Minimal data layer using only current RPC capabilities.

### Poll Loop

```typescript
class KoraPoller {
  private lastBlock = 0n;
  private interval: number;

  constructor(
    private rpcUrl: string,
    private bus: EventBus,
  ) {}

  start() {
    // Block check: every 1s (matches block time)
    this.interval = setInterval(() => this.pollBlock(), 1000);

    // Fee history: every 5s (doesn't change often)
    setInterval(() => this.pollFees(), 5000);
  }

  async pollBlock() {
    const num = await this.rpc("eth_blockNumber");
    if (num <= this.lastBlock) return; // no new block

    // Fetch all new blocks (handles >1 block gap)
    for (let n = this.lastBlock + 1n; n <= num; n++) {
      const block = await this.rpc("eth_getBlockByNumber", [hex(n), true]);
      this.bus.emit("block:new", block);

      for (const tx of block.transactions) {
        this.bus.emit("tx:confirmed", tx);
        this.bus.emit("address:seen", tx.from);
        if (tx.to) this.bus.emit("address:seen", tx.to);
      }
    }

    this.lastBlock = num;
  }

  async pollFees() {
    const history = await this.rpc("eth_feeHistory", ["0x14", "latest", [25, 50, 75]]);
    this.bus.emit("fee:update", history);
  }
}
```

### On-Demand Fetches

Not polled — triggered by user interaction:

```typescript
// User clicks an address
async function loadAddress(addr: string) {
  const [balance, nonce, code] = await Promise.all([
    rpc("eth_getBalance", [addr, "latest"]),
    rpc("eth_getTransactionCount", [addr, "latest"]),
    rpc("eth_getCode", [addr, "latest"]),
  ]);
  return { addr, balance, nonce, isContract: code !== "0x" };
}

// User clicks a transaction
async function loadTxDetail(hash: string) {
  const [tx, receipt] = await Promise.all([
    rpc("eth_getTransactionByHash", [hash]),
    rpc("eth_getTransactionReceipt", [hash]),
  ]);
  return { tx, receipt };
}

// User searches logs
async function searchLogs(filter: LogFilter) {
  return rpc("eth_getLogs", [filter]);
}
```

---

## Phase 2: WebSocket Subscriptions (After RPC Enablement)

Replace polling with push. Same EventBus interface, different source.

```typescript
class KoraSubscriber {
  private ws: WebSocket;

  constructor(
    private wsUrl: string,
    private bus: EventBus,
  ) {}

  async connect() {
    this.ws = new WebSocket(this.wsUrl);

    this.ws.onopen = () => {
      // Subscribe to everything
      this.subscribe("newHeads");
      this.subscribe("newPendingTransactions");
      this.subscribe("logs", {});
    };

    this.ws.onmessage = (msg) => {
      const data = JSON.parse(msg.data);
      if (data.method !== "eth_subscription") return;

      const sub = this.subscriptions.get(data.params.subscription);
      switch (sub.type) {
        case "newHeads":
          // Fetch full block (header doesn't include tx bodies)
          this.fetchAndEmitBlock(data.params.result.number);
          break;

        case "newPendingTransactions":
          this.bus.emit("tx:pending", data.params.result);
          break;

        case "logs":
          this.bus.emit("log:new", data.params.result);
          break;
      }
    };
  }
}
```

### What Changes Visually with Subscriptions

| Event | Polling (Phase 1) | Subscriptions (Phase 2) |
|-------|-------------------|------------------------|
| New block | Up to 1s latency | Instant |
| New transaction | Only seen after inclusion | Seen at mempool entry, then at inclusion |
| Log event | Must poll `eth_getLogs` | Instant push |
| Chain state | Sampled | Continuous |

The visual difference: with polling, the explorer *reacts*. With subscriptions, it *anticipates*.

---

## Data → Visual Mappings

### Block → Terrain Tile

```
block.hash          → heightmap seed (primary visual)
block.gasUsed       → terrain roughness multiplier
block.transactions  → vertical light beams on tile
block.stateRoot     → color palette shift (if different from parent)
block.baseFeePerGas → ambient light intensity
block.timestamp     → x-position (1 tile per second)
```

### Block → Waterfall Card

```
block.gasUsed / gasLimit → card height (4px empty → 60px full)
block.hash              → internal barcode pattern
block.transactions      → horizontal lines within card
len(transactions)       → left accent bar brightness
block.number            → label text
```

### Block → Mosaic Tile

```
block.hash[0..8]   → pattern algorithm (8 possible patterns)
block.hash[8..16]  → color variation within rose spectrum
block.hash[16..24] → symmetry (bilateral, radial, none)
block.hash[24..32] → density/fill
block.gasUsed      → opacity (empty blocks = ghost, full = vivid)
```

### Transaction → Constellation Arc

```
tx.from         → source node position (deterministic from address hash)
tx.to           → target node position
tx.value        → arc curvature (higher value = wider arc)
                → particle count (8-32)
tx.gasPrice     → particle brightness
tx.gasUsed      → arc lifetime before fade
tx.type         → arc color (transfer=bone, create=rose, call=dream)
receipt.status  → completion animation (success=crystallize, fail=shatter)
```

### Transaction → Pending State (Phase 2 only)

```
tx enters mempool  → particle spawns at sender, dream-bright, orbiting
time in mempool    → orbit radius grows (waiting longer = wider orbit)
tx.gasPrice        → particle brightness (higher fee = brighter)
tx included        → orbit snaps to arc, color shifts dream→bone
tx dropped         → particle fades, text-ghost, dissolves
```

### Fee History → Waveform

```
feeHistory.baseFeePerGas  → waveform Y values
feeHistory.gasUsedRatio   → waveform opacity/fill
feeHistory.reward[25]     → lower bound line
feeHistory.reward[75]     → upper bound line
feeHistory.oldestBlock    → x-axis start
```

Rendered as a layered area chart:
- Fill: warning color at 0.15 opacity (burnt amber)
- Line: warning color at full opacity
- 25th/75th percentile as faint bounds

### Address → Constellation Node

```
address bytes[0..4]    → spiral angle (golden ratio distribution)
address bytes[4..8]    → radius from center
transaction_count      → node size (logarithmic, 2px-8px)
balance                → ring brightness (bone spectrum)
is_contract            → shape (circle=EOA, diamond=contract)
last_active_block      → pulse rate (recent=fast, old=slow, very old=none)
```

---

## Caching Strategy

```typescript
class ExplorerCache {
  // LRU cache of full blocks — 500 most recent
  blocks: LRUMap<bigint, Block>;

  // Set of all known addresses — grows monotonically
  addresses: Map<string, AddressMeta>;

  // Ring buffer of fee history — last 1000 data points
  feeHistory: RingBuffer<FeeDataPoint>;

  // Pending transactions (Phase 2) — evicted on inclusion/timeout
  pending: Map<string, PendingTx>;
}
```

Memory budget: ~50MB for 500 blocks with full transactions.
Address set grows unbounded but each entry is <100 bytes.

---

## Fallback Behavior

When RPC is unreachable:

1. **First 3 seconds:** Continue rendering with cached data, no visual change
2. **After 3 seconds:** Status LED shifts to warning (amber pulse)
3. **After 10 seconds:** Status LED shifts to danger (red), "DISCONNECTED" label
4. **Visuals:** Terrain freezes (no new tiles), constellation dims, waterfall stops falling
5. **On reconnect:** Backfill missed blocks, fast-forward terrain/waterfall to catch up, LED returns to green

The explorer should never crash or blank-screen on connection loss. It degrades gracefully
into a frozen but still beautiful state.

---

## TypeScript Type Definitions

```typescript
// apps/explorer/src/data/types.ts

/** Block as returned by eth_getBlockByNumber with full transactions */
export interface ChainBlock {
  number: bigint;
  hash: `0x${string}`;
  parentHash: `0x${string}`;
  stateRoot: `0x${string}`;
  transactionsRoot: `0x${string}`;
  receiptsRoot: `0x${string}`;
  gasLimit: bigint;
  gasUsed: bigint;
  baseFeePerGas: bigint;
  timestamp: bigint;
  transactions: ChainTransaction[];
  // Computed locally, not from RPC:
  _activityLevel: number;  // gasUsed / gasLimit, 0-1
  _arrivalTime: number;    // Date.now() when we received it
}

export interface ChainTransaction {
  hash: `0x${string}`;
  from: `0x${string}`;
  to: `0x${string}` | null;   // null = contract creation
  value: bigint;
  gasPrice: bigint;
  gas: bigint;
  nonce: number;
  input: `0x${string}`;
  blockNumber: bigint;
  blockHash: `0x${string}`;
  transactionIndex: number;
  type: number;                // 0=legacy, 1=2930, 2=1559
}

export interface ChainReceipt {
  transactionHash: `0x${string}`;
  blockHash: `0x${string}`;
  blockNumber: bigint;
  from: `0x${string}`;
  to: `0x${string}` | null;
  gasUsed: bigint;
  cumulativeGasUsed: bigint;
  status: 'success' | 'reverted';
  logs: ChainLog[];
  contractAddress: `0x${string}` | null;
}

export interface ChainLog {
  address: `0x${string}`;
  topics: `0x${string}`[];
  data: `0x${string}`;
  blockNumber: bigint;
  transactionHash: `0x${string}`;
  logIndex: number;
}

export interface AddressMeta {
  address: `0x${string}`;
  balance: bigint;
  nonce: number;
  isContract: boolean;
  firstSeen: bigint;          // block number
  lastSeen: bigint;           // block number
  txCount: number;            // local count (from cached blocks only)
  // Constellation-specific:
  position: { x: number; y: number; z: number };  // deterministic from address
}

export interface FeeDataPoint {
  blockNumber: bigint;
  baseFee: bigint;
  gasUsedRatio: number;
  reward25: bigint;
  reward50: bigint;
  reward75: bigint;
}

export interface PendingTransaction {
  hash: `0x${string}`;
  from: `0x${string}`;
  to: `0x${string}` | null;
  value: bigint;
  gasPrice: bigint;
  submittedAt: number;       // Date.now() when seen in mempool
  status: 'pending' | 'included' | 'dropped';
}

/** Events emitted on the EventBus */
export type BusEvents = {
  'block:new': ChainBlock;
  'tx:confirmed': ChainTransaction;
  'tx:pending': PendingTransaction;
  'tx:resolved': { hash: `0x${string}`; status: 'included' | 'dropped' };
  'fee:update': FeeDataPoint[];
  'address:seen': `0x${string}`;
  'log:new': ChainLog;
  'connection:status': 'connected' | 'reconnecting' | 'disconnected';
};
```

---

## EventBus Implementation

```typescript
// apps/explorer/src/data/bus.ts
import mitt from 'mitt';
import type { BusEvents } from './types';

/** Typed event bus — single instance for the entire app */
export const bus = mitt<BusEvents>();

// Usage:
// bus.emit('block:new', block);
// bus.on('block:new', (block) => { ... });
// const unsub = bus.on('tx:pending', handler); // returns cleanup fn
```

mitt is chosen because it's 200 bytes gzipped, fully typed, and has no dependencies. The EventBus is a thin coordination layer — it does NOT hold state. State lives in the Zustand store; the bus just routes events from the data layer to consumers (scenes, panels, store).

---

## Zustand Store Architecture

Two separate stores to avoid unnecessary re-renders:

```typescript
// apps/explorer/src/data/store.ts
import { create } from 'zustand';
import { LRUMap } from './cache';

/** Chain data store — updated by poller/subscriber, consumed by scenes/panels */
export const useChainStore = create<ChainState>((set, get) => ({
  // ── Connection ──
  status: 'disconnected' as 'connected' | 'reconnecting' | 'disconnected',
  chainId: 0,
  latestBlockNumber: 0n,

  // ── Blocks ──
  blocks: new LRUMap<bigint, ChainBlock>(500),
  latestBlock: null as ChainBlock | null,

  // ── Addresses ──
  addresses: new Map<string, AddressMeta>(),

  // ── Fee History ──
  feeHistory: [] as FeeDataPoint[], // ring buffer, last 1000 points

  // ── Pending Txs (Phase 2) ──
  pendingTxs: new Map<string, PendingTransaction>(),

  // ── Actions ──
  addBlock: (block: ChainBlock) => {
    const state = get();
    state.blocks.set(block.number, block);
    set({
      latestBlock: block,
      latestBlockNumber: block.number,
    });
  },

  trackAddress: (addr: string, meta: Partial<AddressMeta>) => {
    const state = get();
    const existing = state.addresses.get(addr);
    state.addresses.set(addr, { ...existing, ...meta } as AddressMeta);
    set({ addresses: new Map(state.addresses) });
  },

  setConnectionStatus: (status: ChainState['status']) => set({ status }),
}));

/** UI state store — mode, search, detail view */
export const useUIStore = create<UIState>((set) => ({
  mode: 'terrain' as 'terrain' | 'mosaic' | 'detail',
  ambientMode: false,         // true after 30s idle
  ambientOpacity: 1,          // fades to 0.2
  searchOpen: false,
  searchQuery: '',
  detailView: null as DetailTarget | null,
  paused: false,              // space bar toggle

  setMode: (mode) => set({ mode }),
  togglePause: () => set((s) => ({ paused: !s.paused })),
  openDetail: (target) => set({ detailView: target, mode: 'detail' }),
  closeDetail: () => set({ detailView: null }),
  setAmbient: (active) => set({ ambientMode: active, ambientOpacity: active ? 0.2 : 1 }),
}));

type DetailTarget =
  | { type: 'block'; number: bigint }
  | { type: 'tx'; hash: `0x${string}` }
  | { type: 'address'; address: `0x${string}` };
```

---

## RPC Client Setup

```typescript
// apps/explorer/src/data/rpc.ts
import { createPublicClient, http, webSocket } from 'viem';
import { defineChain } from 'viem/chains';

const kora = defineChain({
  id: 1337,
  name: 'Kora',
  nativeCurrency: { name: 'Ether', symbol: 'ETH', decimals: 18 },
  rpcUrls: {
    default: {
      http: [import.meta.env.VITE_RPC_HTTP ?? 'http://65.109.61.210:8545'],
      webSocket: [import.meta.env.VITE_RPC_WS ?? 'ws://65.109.61.210:8546'],
    },
  },
});

/** HTTP client — used for polling (Phase 1) and on-demand queries */
export const httpClient = createPublicClient({
  chain: kora,
  transport: http(undefined, {
    retryCount: 3,
    retryDelay: 1000,
    timeout: 10_000,
  }),
});

/** WebSocket client — used for subscriptions (Phase 2) */
export function createWsClient() {
  return createPublicClient({
    chain: kora,
    transport: webSocket(undefined, {
      retryCount: Infinity,         // always reconnect
      retryDelay: 1000,             // 1s between attempts
      keepAlive: { interval: 15_000 }, // ping every 15s
    }),
  });
}
```

---

## Connection Management

```typescript
// apps/explorer/src/data/connection.ts

export class ConnectionManager {
  private poller: KoraPoller | null = null;
  private subscriber: KoraSubscriber | null = null;
  private reconnectTimer: number | null = null;
  private reconnectAttempt = 0;

  async connect() {
    try {
      // Phase 1: Always start with polling
      this.poller = new KoraPoller(httpClient, bus);
      await this.poller.start();
      bus.emit('connection:status', 'connected');
      this.reconnectAttempt = 0;

      // Phase 2: Try WebSocket upgrade
      try {
        this.subscriber = new KoraSubscriber(createWsClient(), bus);
        await this.subscriber.connect();
        // If WS works, stop polling for block numbers (WS pushes them)
        this.poller.disableBlockPoll();
        // Keep fee history polling (no WS equivalent)
      } catch {
        // WS not available — polling continues
        console.info('WebSocket not available, using polling only');
      }
    } catch (err) {
      this.scheduleReconnect();
    }
  }

  private scheduleReconnect() {
    bus.emit('connection:status', 'reconnecting');
    // Exponential backoff: 1s, 2s, 4s, 8s, 16s, max 30s
    const delay = Math.min(30_000, 1000 * Math.pow(2, this.reconnectAttempt));
    this.reconnectAttempt++;
    this.reconnectTimer = window.setTimeout(() => this.connect(), delay);
  }

  disconnect() {
    this.poller?.stop();
    this.subscriber?.disconnect();
    if (this.reconnectTimer) clearTimeout(this.reconnectTimer);
    bus.emit('connection:status', 'disconnected');
  }
}
```

### Error Handling Strategy

| Error Type | Behavior |
|------------|----------|
| RPC timeout (10s) | Retry up to 3 times via viem transport config |
| RPC error response | Log, emit on bus, continue polling next cycle |
| WebSocket close | Auto-reconnect (viem handles this), fall back to polling |
| Block gap (missed blocks) | Backfill: fetch all blocks between lastKnown and current |
| Parse error | Log + skip, never crash the poll loop |
| Rate limit (429) | Back off for `Retry-After` header duration, then resume |

---

## Cache Implementation

```typescript
// apps/explorer/src/data/cache.ts

/** LRU cache with O(1) get/set/delete */
export class LRUMap<K, V> {
  private map = new Map<K, V>();

  constructor(private maxSize: number) {}

  get(key: K): V | undefined {
    const val = this.map.get(key);
    if (val !== undefined) {
      // Move to end (most recently used)
      this.map.delete(key);
      this.map.set(key, val);
    }
    return val;
  }

  set(key: K, value: V): void {
    this.map.delete(key); // remove if exists (resets position)
    this.map.set(key, value);
    if (this.map.size > this.maxSize) {
      // Delete oldest (first key)
      const firstKey = this.map.keys().next().value;
      this.map.delete(firstKey);
    }
  }

  has(key: K): boolean { return this.map.has(key); }
  get size(): number { return this.map.size; }

  /** Iterate newest → oldest */
  *values(): IterableIterator<V> {
    const entries = [...this.map.values()];
    for (let i = entries.length - 1; i >= 0; i--) {
      yield entries[i];
    }
  }
}
```

### Memory Budget

| Structure | Max Size | Entry Size | Total |
|-----------|----------|------------|-------|
| Block cache (LRU) | 500 blocks | ~10KB avg (with full tx bodies) | ~5MB |
| Address set | unbounded (grows) | ~120B per entry | ~12MB at 100K addresses |
| Fee history ring | 1000 points | ~64B per point | ~64KB |
| Pending txs (Phase 2) | 5000 txs | ~200B per entry | ~1MB |
| WebGL buffers (all scenes) | varies | — | ~50-100MB GPU |
| **Total (JS heap)** | | | **~20MB typical** |

The 50MB estimate in the original spec was conservative. Actual JS heap usage is lower because we store bigints (8 bytes) rather than hex strings, and the LRU evicts aggressively. GPU memory dominates.

---

## Wiring It All Together

Initialization sequence in `main.tsx`:

```typescript
// 1. Create stores (Zustand — already created as modules)
// 2. Create EventBus (mitt — already created as module)

// 3. Wire EventBus → Zustand store
bus.on('block:new', (block) => {
  useChainStore.getState().addBlock(block);
  // Update activity CSS variable for rose wash
  document.documentElement.style.setProperty(
    '--rd-activity',
    String(block._activityLevel * 0.15)
  );
});

bus.on('connection:status', (status) => {
  useChainStore.getState().setConnectionStatus(status);
});

// 4. Scenes subscribe to EventBus directly (not through React)
//    This avoids re-render overhead for 60fps animation data.
//    Scene components use bus.on() in useEffect, not Zustand selectors.

// 5. Panels subscribe to Zustand store (React-friendly)
//    Panels re-render when store data changes via selectors.

// 6. Start connection
const connection = new ConnectionManager();
connection.connect();
```

This separation is critical: **scenes use the EventBus** (imperative, no React re-renders), **panels use Zustand** (declarative, React re-renders on state change). The bus feeds both paths from a single data source.

---

## Consensus Data Flow

The consensus data follows a separate path from chain data. While block/tx data flows through the Ethereum RPC layer (`eth_*`), consensus data flows through kora-specific subscriptions.

### Architecture

```
                    ┌─────────────────────────┐
                    │ Simplex Consensus Engine │
                    └──────────┬──────────────┘
                               │ Activity events (all 10 types)
                    ┌──────────▼──────────────┐
                    │   ConsensusReporter      │
                    │   (new Reporter impl)    │
                    │                          │
                    │   tokio::broadcast       │
                    │   channel (cap: 1024)    │
                    └──────────┬──────────────┘
                               │
                    ┌──────────▼──────────────┐
                    │   kora_subscribe         │
                    │   ("consensus")          │
                    │                          │
                    │   Filters by sub type:   │
                    │   consensus.rounds       │
                    │   consensus.votes        │
                    │   consensus.safety       │
                    └──────────┬──────────────┘
                               │ WebSocket push
                    ┌──────────▼──────────────┐
                    │   Explorer (browser)     │
                    │                          │
                    │   ConsensusRound         │
                    │   state machine          │
                    │         │                │
                    │   ┌─────▼─────┐          │
                    │   │ Consensus │          │
                    │   │ Ring      │          │
                    │   │ (Scene 4) │          │
                    │   └───────────┘          │
                    └──────────────────────────┘
```

### TypeScript Types for Consensus

```typescript
// apps/explorer/src/data/consensus.ts

/** Wire format from kora_subscribe("consensus") */
export type ConsensusEvent =
  | { type: 'notarize'; view: number; payload: string; timestampMs: number }
  | { type: 'notarization'; view: number; payload: string; seed: string; timestampMs: number }
  | { type: 'certification'; view: number; payload: string; timestampMs: number }
  | { type: 'finalize'; view: number; payload: string; timestampMs: number }
  | { type: 'finalization'; view: number; payload: string; seed: string; blockHeight: number | null; timestampMs: number }
  | { type: 'nullify'; timestampMs: number }
  | { type: 'nullification'; view: number; timestampMs: number }
  | { type: 'conflictingNotarize'; timestampMs: number; severity: 'critical' }
  | { type: 'conflictingFinalize'; timestampMs: number; severity: 'critical' }
  | { type: 'nullifyFinalize'; timestampMs: number; severity: 'critical' };

/** Local state machine tracking the current consensus round */
export interface ConsensusRound {
  view: number;
  leader: number;               // view % validatorCount
  phase: RoundPhase;
  payload: string | null;       // block digest being proposed
  startedAt: number;            // timestamp when view began

  // Phase timestamps (filled as events arrive)
  notarizeAt: number | null;
  notarizationAt: number | null;
  certificationAt: number | null;
  finalizeAt: number | null;
  finalizationAt: number | null;
  nullifyAt: number | null;
  nullificationAt: number | null;
}

export type RoundPhase =
  | 'proposing'
  | 'notarizing'
  | 'certifying'
  | 'finalizing'
  | 'finalized'
  | 'nullifying'
  | 'nullified';

/** Safety violation record — persists in UI */
export interface SafetyEvent {
  type: 'conflictingNotarize' | 'conflictingFinalize' | 'nullifyFinalize';
  timestampMs: number;
  acknowledged: boolean;   // user must dismiss
}
```

### Consensus → EventBus Mapping

```typescript
// Extended BusEvents type
export type BusEvents = {
  // ... existing chain events ...

  // Consensus events (from kora_subscribe)
  'consensus:event': ConsensusEvent;
  'consensus:round': ConsensusRound;        // emitted when round phase changes
  'consensus:safety': SafetyEvent;          // emitted on safety violation

  // Mempool events (from kora_subscribe("mempool"))
  'mempool:txAdded': { hash: string; from: string; to: string | null; value: string; timestampMs: number };
  'mempool:txIncluded': { hash: string; blockHeight: number; timestampMs: number };
  'mempool:txEvicted': { hash: string; reason: string; timestampMs: number };
};
```

### Consensus Zustand Store

```typescript
// apps/explorer/src/data/store.ts (additions)

export const useConsensusStore = create<ConsensusState>((set, get) => ({
  // Current round
  currentRound: null as ConsensusRound | null,

  // History (last 200 rounds)
  roundHistory: [] as ConsensusRound[],

  // Aggregate metrics
  avgFinalizationMs: 0,
  nullificationRate: 0,       // 0-1

  // Safety
  safetyViolations: [] as SafetyEvent[],
  unacknowledgedViolations: 0,

  // Validator info
  validatorCount: 4,          // from kora_consensusState
  threshold: 3,               // from kora_consensusState
  validatorIndex: 0,          // this node's index

  // Connection
  subscribed: false,

  // Actions
  processEvent: (event: ConsensusEvent) => {
    const state = get();
    // Update currentRound based on event type
    // Push to history on finalization/nullification
    // Calculate rolling averages
  },

  acknowledgeSafetyViolation: (index: number) => {
    // Mark violation as acknowledged (removes alert)
  },
}));
```

### Poll Fallback

If WebSocket is unavailable, consensus state can be polled via `kora_consensusState`:

```typescript
// Fallback polling for consensus (2s interval)
setInterval(async () => {
  if (wsConnected) return; // WS handles it
  const state = await rpc.request('kora_consensusState');
  useConsensusStore.getState().updateFromPoll(state);
}, 2000);
```

The poll provides cumulative metrics but not individual event timing. The consensus ring visualization degrades gracefully: it shows round outcomes (finalized/nullified) but not the vote-by-vote progression within each round.
