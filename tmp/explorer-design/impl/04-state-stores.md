# Implementation: State Stores & Event Bus

Implementation spec for the Zustand state stores and mitt event bus that form the
reactive data backbone of the Kora Explorer.

**Source design:** [04-data-flows.md](../04-data-flows.md)
**Path prefix:** `apps/explorer/src/`

---

## Architecture

Two communication patterns serve different consumers:

```
                         ┌────────────────────────┐
                         │   DataLayer             │
                         │   (Poller / Subscriber) │
                         └────────────┬───────────┘
                                      │
                              bus.emit(event)
                                      │
                         ┌────────────▼───────────┐
                         │       Event Bus         │
                         │       (mitt)            │
                         │                         │
                         │  imperative             │
                         │  fire-and-forget        │
                         │  SOURCE OF TRUTH for    │
                         │  real-time data         │
                         └──┬──────────────────┬──┘
                            │                  │
               bus.on()     │                  │   bus.on() in store init
               in useEffect │                  │
                            │                  │
                ┌───────────▼──┐    ┌──────────▼──────────┐
                │  WebGL       │    │  Zustand Stores      │
                │  Scenes      │    │  (chain, ui,         │
                │              │    │   consensus)          │
                │  No React    │    │                      │
                │  re-renders  │    │  Declarative,        │
                │              │    │  reactive             │
                └──────────────┘    └──────────┬──────────┘
                                               │
                                    useStore(selector)
                                               │
                                    ┌──────────▼──────────┐
                                    │  React Panels        │
                                    │  (sidebar, search,   │
                                    │   detail, status)    │
                                    └─────────────────────┘
```

**Rule:** The bus is the source of truth for real-time data. Zustand stores subscribe
to the bus and derive state. Scenes subscribe to the bus directly and bypass React
entirely. Panels subscribe to Zustand stores through selectors.

---

## File Map

| File | Purpose |
|------|---------|
| `src/bus/events.ts` | BusEvents type map + singleton mitt instance |
| `src/stores/chain.ts` | Block ring buffer, tx/receipt/address LRU caches, fee history |
| `src/stores/ui.ts` | Active scene, selection, search, ambient mode, performance tier |
| `src/stores/consensus.ts` | Round state machine, round history, safety violations |
| `src/stores/index.ts` | Store initialization, bus-to-store wiring, combined selectors |
| `src/lib/lru.ts` | Generic LRU map (used by chain and consensus stores) |

---

## 1. Event Bus (`src/bus/events.ts`)

The bus carries every event type the system produces. Scenes and stores subscribe
to the specific events they care about.

```typescript
// src/bus/events.ts

import mitt from 'mitt';
import type {
  ChainBlock,
  ChainTransaction,
  ChainReceipt,
  ChainLog,
  AddressMeta,
  FeeDataPoint,
} from '@/data/types';
import type { ConsensusEvent, ConsensusRound, SafetyEvent } from '@/data/consensus';

// ---------------------------------------------------------------------------
// Scene name enum
// ---------------------------------------------------------------------------

export type SceneName = 'terrain' | 'constellation' | 'waterfall' | 'consensus';

// ---------------------------------------------------------------------------
// Connection state
// ---------------------------------------------------------------------------

export type ConnectionState = 'connected' | 'reconnecting' | 'disconnected';

// ---------------------------------------------------------------------------
// Search result
// ---------------------------------------------------------------------------

export interface SearchResult {
  type: 'block' | 'transaction' | 'address';
  label: string;
  subtitle: string;
  source: 'cache' | 'rpc';
  navigateTo: string;
}

// ---------------------------------------------------------------------------
// Node status (from kora_consensusState poll)
// ---------------------------------------------------------------------------

export interface NodeStatus {
  chainId: number;
  validatorIndex: number;
  validatorCount: number;
  threshold: number;
  currentView: number;
  currentLeader: number;
  finalizedCount: number;
  nullifiedCount: number;
  peerCount: number;
  uptimeSecs: number;
  isLeader: boolean;
  lastFinalizedAtMs: number | null;
  avgFinalizationMs: number | null;
  safetyViolations: number;
}

// ---------------------------------------------------------------------------
// Validator info (derived from NodeStatus at connection time)
// ---------------------------------------------------------------------------

export interface ValidatorInfo {
  index: number;
  isLocal: boolean;
}

// ---------------------------------------------------------------------------
// Bus event map
// ---------------------------------------------------------------------------

export type BusEvents = {
  // Chain events (from DataLayer poller/subscriber)
  'block:new': ChainBlock;
  'block:reorg': { oldHead: bigint; newHead: bigint };
  'tx:new': ChainTransaction;
  'tx:confirmed': { hash: `0x${string}`; receipt: ChainReceipt };
  'tx:pending': ChainTransaction;
  'log:new': ChainLog;

  // Consensus events (from kora_subscribe("consensus"))
  'consensus:event': ConsensusEvent;
  'consensus:round': ConsensusRound;
  'node:status': NodeStatus;

  // Connection state
  'connection:state': ConnectionState;

  // Navigation / UI events
  'scene:navigate': SceneName;
  'entity:select': { type: 'block' | 'tx' | 'address'; id: string };
  'entity:deselect': void;

  // Search
  'search:query': string;
  'search:result': SearchResult[];
};

// ---------------------------------------------------------------------------
// Singleton bus instance
// ---------------------------------------------------------------------------

export const bus = mitt<BusEvents>();

// ---------------------------------------------------------------------------
// Development helpers
// ---------------------------------------------------------------------------

if (import.meta.env.DEV) {
  // Log every event in development for debugging.
  // Wrap in a check so the wildcard handler is tree-shaken in production.
  bus.on('*', (type, payload) => {
    console.debug(`[bus] ${type as string}`, payload);
  });
}
```

### Usage pattern for scenes

Scenes subscribe to the bus inside `useEffect` and clean up on unmount. This keeps
scene updates entirely outside React's render cycle.

```typescript
// Example: terrain scene subscribing to new blocks
// src/scenes/terrain/TerrainScene.tsx

import { useEffect, useRef } from 'react';
import { bus } from '@/bus/events';
import type { ChainBlock } from '@/data/types';

export function useTerrainBus(terrainRing: TerrainRing) {
  // Ref avoids stale closure over the ring instance.
  const ringRef = useRef(terrainRing);
  ringRef.current = terrainRing;

  useEffect(() => {
    const handleBlock = (block: ChainBlock) => {
      // Imperative update -- no setState, no re-render.
      ringRef.current.addBlock(block);
    };

    const handleReorg = (reorg: { oldHead: bigint; newHead: bigint }) => {
      ringRef.current.rollback(reorg.oldHead, reorg.newHead);
    };

    bus.on('block:new', handleBlock);
    bus.on('block:reorg', handleReorg);

    return () => {
      bus.off('block:new', handleBlock);
      bus.off('block:reorg', handleReorg);
    };
  }, []);
}
```

Key rules:
- Always call `bus.off()` in the cleanup return of `useEffect`.
- Never call `setState` inside a bus handler used by a scene. That would drag the
  event back into React's render cycle and defeat the purpose.
- For panel components that need bus data, let the Zustand store subscribe to the
  bus and read the store via selectors.

---

## 2. LRU Map (`src/lib/lru.ts`)

A generic LRU cache used by the chain and consensus stores. Backed by a plain `Map`
whose insertion order provides the eviction order.

```typescript
// src/lib/lru.ts

/**
 * LRU cache with O(1) get/set/delete.
 *
 * Uses the native Map insertion order as the recency list:
 * - get() deletes and re-inserts to move the entry to the tail (most recent).
 * - set() evicts the head (oldest) entry when the map exceeds capacity.
 */
export class LRUMap<K, V> {
  private map = new Map<K, V>();

  constructor(private readonly capacity: number) {
    if (capacity < 1) {
      throw new RangeError(`LRUMap capacity must be >= 1, got ${capacity}`);
    }
  }

  /** Get a value by key. Returns undefined if not present. Promotes key to most-recent. */
  get(key: K): V | undefined {
    const val = this.map.get(key);
    if (val === undefined) return undefined;

    // Move to tail (most recently used).
    this.map.delete(key);
    this.map.set(key, val);
    return val;
  }

  /** Insert or update a key. Evicts the oldest entry if at capacity. */
  set(key: K, value: V): void {
    // Delete first so re-insertion moves to tail even for updates.
    this.map.delete(key);
    this.map.set(key, value);

    if (this.map.size > this.capacity) {
      // Map iterator yields in insertion order; first key is the oldest.
      const oldest = this.map.keys().next().value!;
      this.map.delete(oldest);
    }
  }

  /** Check membership without promoting recency. */
  has(key: K): boolean {
    return this.map.has(key);
  }

  /** Delete a key. Returns true if the key existed. */
  delete(key: K): boolean {
    return this.map.delete(key);
  }

  /** Number of entries currently held. */
  get size(): number {
    return this.map.size;
  }

  /** Iterate values from newest to oldest. */
  *values(): IterableIterator<V> {
    const entries = [...this.map.values()];
    for (let i = entries.length - 1; i >= 0; i--) {
      yield entries[i]!;
    }
  }

  /** Iterate entries from newest to oldest. */
  *entries(): IterableIterator<[K, V]> {
    const entries = [...this.map.entries()];
    for (let i = entries.length - 1; i >= 0; i--) {
      yield entries[i]!;
    }
  }

  /** Iterate keys from newest to oldest. */
  *keys(): IterableIterator<K> {
    const keys = [...this.map.keys()];
    for (let i = keys.length - 1; i >= 0; i--) {
      yield keys[i]!;
    }
  }

  /** Remove all entries. */
  clear(): void {
    this.map.clear();
  }

  /** Snapshot the current entries as a plain array (newest first). Used for devtools serialization. */
  toArray(): Array<[K, V]> {
    return [...this.entries()];
  }
}
```

---

## 3. Chain Store (`src/stores/chain.ts`)

The primary data store. Holds the last 128 blocks in a ring buffer, LRU caches for
transactions/receipts/addresses, and a sliding window of fee data points.

```typescript
// src/stores/chain.ts

import { create } from 'zustand';
import { devtools } from 'zustand/middleware';
import { immer } from 'zustand/middleware/immer';
import { LRUMap } from '@/lib/lru';
import type {
  ChainBlock,
  ChainTransaction,
  ChainReceipt,
  AddressMeta,
  FeeDataPoint,
} from '@/data/types';
import type { ConnectionState } from '@/bus/events';

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/** Maximum blocks held in the ring buffer. */
const MAX_BLOCKS = 128;

/** Maximum cached transactions (LRU). */
const MAX_TRANSACTIONS = 2048;

/** Maximum cached receipts (LRU). */
const MAX_RECEIPTS = 1024;

/** Maximum cached address metadata entries (LRU). */
const MAX_ADDRESSES = 512;

/** Maximum fee data points retained. */
const MAX_FEE_HISTORY = 64;

// ---------------------------------------------------------------------------
// State shape
// ---------------------------------------------------------------------------

export interface ChainState {
  // -- Block ring buffer (last MAX_BLOCKS blocks) --
  blocks: Map<bigint, ChainBlock>;
  blockOrder: bigint[]; // ordered newest-first
  latestBlock: bigint;

  // -- Transaction cache (LRU) --
  transactions: LRUMap<`0x${string}`, ChainTransaction>;

  // -- Receipt cache (LRU) --
  receipts: LRUMap<`0x${string}`, ChainReceipt>;

  // -- Address metadata cache (LRU) --
  addresses: LRUMap<`0x${string}`, AddressMeta>;

  // -- Fee history (sliding window, last MAX_FEE_HISTORY blocks) --
  feeHistory: FeeDataPoint[];

  // -- Connection --
  connectionState: ConnectionState;
  rpcLatency: number; // rolling average in ms

  // -- Actions --
  addBlock: (block: ChainBlock) => void;
  handleReorg: (oldHead: bigint, newHead: bigint) => void;
  addTransaction: (tx: ChainTransaction) => void;
  addReceipt: (hash: `0x${string}`, receipt: ChainReceipt) => void;
  addAddressMeta: (meta: AddressMeta) => void;
  pushFeeData: (points: FeeDataPoint[]) => void;
  setConnectionState: (state: ConnectionState) => void;
  updateRpcLatency: (sample: number) => void;
}

// ---------------------------------------------------------------------------
// Store creation
// ---------------------------------------------------------------------------

export const useChainStore = create<ChainState>()(
  devtools(
    immer((set, get) => ({
      // ── Initial state ────────────────────────────────────────────
      blocks: new Map<bigint, ChainBlock>(),
      blockOrder: [] as bigint[],
      latestBlock: 0n,

      transactions: new LRUMap<`0x${string}`, ChainTransaction>(MAX_TRANSACTIONS),
      receipts: new LRUMap<`0x${string}`, ChainReceipt>(MAX_RECEIPTS),
      addresses: new LRUMap<`0x${string}`, AddressMeta>(MAX_ADDRESSES),

      feeHistory: [] as FeeDataPoint[],

      connectionState: 'disconnected' as ConnectionState,
      rpcLatency: 0,

      // ── Actions ──────────────────────────────────────────────────

      addBlock: (block: ChainBlock) => {
        set((draft) => {
          // Insert block into map.
          draft.blocks.set(block.number, block);

          // Maintain blockOrder newest-first.
          draft.blockOrder.unshift(block.number);

          // Update latest block number.
          if (block.number > draft.latestBlock) {
            draft.latestBlock = block.number;
          }

          // Ring buffer eviction: remove oldest when over capacity.
          while (draft.blockOrder.length > MAX_BLOCKS) {
            const evicted = draft.blockOrder.pop()!;
            draft.blocks.delete(evicted);
          }

          // Index transactions from the block into the tx cache.
          for (const tx of block.transactions) {
            draft.transactions.set(tx.hash, tx);
          }

          // Update activity CSS variable for rose wash.
          if (typeof document !== 'undefined') {
            document.documentElement.style.setProperty(
              '--rd-activity',
              String(block._activityLevel * 0.15),
            );
          }
        });
      },

      handleReorg: (oldHead: bigint, newHead: bigint) => {
        set((draft) => {
          // Remove all blocks above the new head (they are no longer canonical).
          const toRemove: bigint[] = [];
          for (const num of draft.blockOrder) {
            if (num > newHead) {
              toRemove.push(num);
            }
          }
          for (const num of toRemove) {
            draft.blocks.delete(num);
          }
          draft.blockOrder = draft.blockOrder.filter((n) => n <= newHead);
          draft.latestBlock = newHead;
        });
      },

      addTransaction: (tx: ChainTransaction) => {
        set((draft) => {
          draft.transactions.set(tx.hash, tx);
        });
      },

      addReceipt: (hash: `0x${string}`, receipt: ChainReceipt) => {
        set((draft) => {
          draft.receipts.set(hash, receipt);
        });
      },

      addAddressMeta: (meta: AddressMeta) => {
        set((draft) => {
          draft.addresses.set(meta.address, meta);
        });
      },

      pushFeeData: (points: FeeDataPoint[]) => {
        set((draft) => {
          draft.feeHistory.push(...points);
          // Trim to sliding window.
          if (draft.feeHistory.length > MAX_FEE_HISTORY) {
            draft.feeHistory = draft.feeHistory.slice(-MAX_FEE_HISTORY);
          }
        });
      },

      setConnectionState: (state: ConnectionState) => {
        set((draft) => {
          draft.connectionState = state;
        });
      },

      updateRpcLatency: (sample: number) => {
        set((draft) => {
          // Exponential moving average (alpha = 0.2).
          draft.rpcLatency = draft.rpcLatency === 0
            ? sample
            : draft.rpcLatency * 0.8 + sample * 0.2;
        });
      },
    })),
    { name: 'chain-store', enabled: import.meta.env.DEV },
  ),
);

// ---------------------------------------------------------------------------
// Selectors
//
// Each selector extracts a minimal slice so that Zustand's default shallow
// equality check prevents re-renders when unrelated state changes.
// ---------------------------------------------------------------------------

/** Latest block number as bigint. */
export const useLatestBlockNumber = () =>
  useChainStore((s) => s.latestBlock);

/** Full latest ChainBlock object (or undefined if no blocks yet). */
export const useLatestBlock = () =>
  useChainStore((s) => {
    if (s.latestBlock === 0n) return undefined;
    return s.blocks.get(s.latestBlock);
  });

/** A specific block by number. Returns undefined if not in the ring buffer. */
export const useBlock = (number: bigint) =>
  useChainStore((s) => s.blocks.get(number));

/** Ordered block numbers, newest first. For iteration in panels. */
export const useBlockOrder = () =>
  useChainStore((s) => s.blockOrder);

/** A specific transaction by hash. Returns undefined if not cached. */
export const useTransaction = (hash: `0x${string}`) =>
  useChainStore((s) => s.transactions.get(hash));

/** A specific receipt by tx hash. Returns undefined if not cached. */
export const useReceipt = (hash: `0x${string}`) =>
  useChainStore((s) => s.receipts.get(hash));

/** Address metadata by address. Returns undefined if not cached. */
export const useAddressMeta = (address: `0x${string}`) =>
  useChainStore((s) => s.addresses.get(address));

/** Current fee history array (last MAX_FEE_HISTORY data points). */
export const useFeeHistory = () =>
  useChainStore((s) => s.feeHistory);

/** Connection state string. */
export const useConnectionState = () =>
  useChainStore((s) => s.connectionState);

/** RPC latency rolling average in ms. */
export const useRpcLatency = () =>
  useChainStore((s) => s.rpcLatency);
```

### Note on immer with Map / LRUMap

Zustand's immer middleware uses structural sharing on plain objects and arrays.
`Map` and custom class instances (like `LRUMap`) are treated as opaque -- immer
wraps them in a draft proxy but does not deeply clone them. This is fine because:

1. We mutate the Map/LRUMap directly inside the `set()` callback.
2. Zustand triggers a re-render for any `set()` call regardless of deep equality.
3. Selectors that extract primitives (like `s.latestBlock`) get proper memoization.

For selectors that return a Map entry (like `useBlock(number)`), the returned object
reference changes only when `set()` is called, which is exactly when we want a
re-render.

---

## 4. UI Store (`src/stores/ui.ts`)

Controls the visual state of the explorer: which scene is active, what entity is
selected, search state, ambient mode, and the performance tier.

```typescript
// src/stores/ui.ts

import { create } from 'zustand';
import { devtools, persist } from 'zustand/middleware';
import type { SceneName, SearchResult } from '@/bus/events';

// ---------------------------------------------------------------------------
// Performance tier detection
// ---------------------------------------------------------------------------

export type PerformanceTier = 'full' | 'standard' | 'light' | 'mobile';

/**
 * Auto-detect the appropriate performance tier based on device capabilities.
 *
 * Checks, in order:
 *   1. Mobile user agent or narrow viewport -> 'mobile'
 *   2. No WebGL2 support -> 'light'
 *   3. Low device memory (< 4 GB) -> 'light'
 *   4. Integrated GPU with < 8 GB memory -> 'standard'
 *   5. Everything else -> 'full'
 */
export function detectPerformanceTier(): PerformanceTier {
  // Guard for SSR / non-browser environments.
  if (typeof window === 'undefined' || typeof navigator === 'undefined') {
    return 'light';
  }

  // Mobile detection.
  if (/Mobi|Android/i.test(navigator.userAgent)) return 'mobile';
  if (window.innerWidth < 760) return 'mobile';

  // WebGL2 probe.
  const canvas = document.createElement('canvas');
  const gl = canvas.getContext('webgl2');
  if (!gl) return 'light';

  // Device memory (non-standard, Chrome-only, returns GB).
  const memory = (navigator as Record<string, unknown>).deviceMemory as
    | number
    | undefined;
  const effectiveMemory = memory ?? 8; // assume 8 GB if API unavailable
  if (effectiveMemory < 4) return 'light';

  // GPU renderer heuristic.
  const debugInfo = gl.getExtension('WEBGL_debug_renderer_info');
  if (debugInfo) {
    const renderer = gl.getParameter(
      debugInfo.UNMASKED_RENDERER_WEBGL,
    ) as string;
    // Integrated GPUs (excluding Intel Arc, which is discrete).
    const isIntegrated =
      /Intel|Mali|Adreno/i.test(renderer) && !/Arc/i.test(renderer);
    if (isIntegrated && effectiveMemory < 8) return 'standard';
  }

  // Clean up the probe canvas.
  canvas.width = 0;
  canvas.height = 0;

  return 'full';
}

// ---------------------------------------------------------------------------
// State shape
// ---------------------------------------------------------------------------

export interface SelectedEntity {
  type: 'block' | 'tx' | 'address';
  id: string;
}

export interface UIState {
  // Scene
  activeScene: SceneName;

  // Selection
  selectedEntity: SelectedEntity | null;

  // Search
  searchQuery: string;
  searchResults: SearchResult[];
  searchOpen: boolean;

  // Ambient mode (set after 30s idle via useAmbientMode hook)
  ambientMode: boolean;

  // Layout
  sidebarCollapsed: boolean;

  // Performance
  performanceTier: PerformanceTier;

  // Actions
  setScene: (scene: SceneName) => void;
  selectEntity: (entity: SelectedEntity | null) => void;
  setSearch: (query: string) => void;
  setSearchResults: (results: SearchResult[]) => void;
  setSearchOpen: (open: boolean) => void;
  toggleAmbient: () => void;
  setAmbient: (active: boolean) => void;
  toggleSidebar: () => void;
  setPerformanceTier: (tier: PerformanceTier) => void;
}

// ---------------------------------------------------------------------------
// Store creation
//
// The `persist` middleware saves `activeScene` and `sidebarCollapsed` to
// localStorage so the user's layout preferences survive page reloads.
// Other state is ephemeral -- search, selection, ambient mode all reset.
// ---------------------------------------------------------------------------

export const useUIStore = create<UIState>()(
  devtools(
    persist(
      (set) => ({
        // ── Initial state ──────────────────────────────────────────
        activeScene: 'terrain' as SceneName,
        selectedEntity: null,
        searchQuery: '',
        searchResults: [] as SearchResult[],
        searchOpen: false,
        ambientMode: false,
        sidebarCollapsed: false,
        performanceTier: detectPerformanceTier(),

        // ── Actions ────────────────────────────────────────────────

        setScene: (scene: SceneName) => {
          set({ activeScene: scene, selectedEntity: null });
        },

        selectEntity: (entity: SelectedEntity | null) => {
          set({ selectedEntity: entity });
        },

        setSearch: (query: string) => {
          set({ searchQuery: query });
        },

        setSearchResults: (results: SearchResult[]) => {
          set({ searchResults: results });
        },

        setSearchOpen: (open: boolean) => {
          set({
            searchOpen: open,
            // Clear results when closing search.
            ...(open ? {} : { searchQuery: '', searchResults: [] }),
          });
        },

        toggleAmbient: () => {
          set((s) => ({ ambientMode: !s.ambientMode }));
        },

        setAmbient: (active: boolean) => {
          set({ ambientMode: active });
        },

        toggleSidebar: () => {
          set((s) => ({ sidebarCollapsed: !s.sidebarCollapsed }));
        },

        setPerformanceTier: (tier: PerformanceTier) => {
          set({ performanceTier: tier });
        },
      }),
      {
        name: 'kora-explorer-ui',
        // Only persist layout preferences, not ephemeral UI state.
        partialize: (state) => ({
          activeScene: state.activeScene,
          sidebarCollapsed: state.sidebarCollapsed,
        }),
      },
    ),
    { name: 'ui-store', enabled: import.meta.env.DEV },
  ),
);

// ---------------------------------------------------------------------------
// Selectors
// ---------------------------------------------------------------------------

export const useActiveScene = () => useUIStore((s) => s.activeScene);

export const useSelectedEntity = () => useUIStore((s) => s.selectedEntity);

export const useSearchState = () =>
  useUIStore((s) => ({
    query: s.searchQuery,
    results: s.searchResults,
    open: s.searchOpen,
  }));

export const useAmbientMode = () => useUIStore((s) => s.ambientMode);

export const useSidebarCollapsed = () => useUIStore((s) => s.sidebarCollapsed);

export const usePerformanceTier = () => useUIStore((s) => s.performanceTier);
```

### Note on `persist` and `partialize`

Only `activeScene` and `sidebarCollapsed` are persisted. Everything else --
search state, selection, ambient mode, performance tier -- resets on page load.
Performance tier is re-detected at store creation time so it picks up hardware
changes (e.g. connecting an external monitor or docking a laptop).

---

## 5. Consensus Store (`src/stores/consensus.ts`)

Tracks the local consensus state machine: current round, round history, validator
set, and safety violations.

```typescript
// src/stores/consensus.ts

import { create } from 'zustand';
import { devtools } from 'zustand/middleware';
import { immer } from 'zustand/middleware/immer';
import type {
  ConsensusEvent,
  ConsensusRound,
  RoundPhase,
  SafetyEvent,
} from '@/data/consensus';
import type { ValidatorInfo, NodeStatus } from '@/bus/events';

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/** Maximum rounds retained in history. */
const MAX_ROUND_HISTORY = 32;

// ---------------------------------------------------------------------------
// State shape
// ---------------------------------------------------------------------------

export interface ConsensusState {
  // Current round being tracked.
  currentRound: ConsensusRound | null;

  // Ring buffer of completed rounds (last MAX_ROUND_HISTORY).
  roundHistory: ConsensusRound[];

  // Validator set (populated from kora_consensusState at connection).
  validators: ValidatorInfo[];
  validatorCount: number;
  threshold: number;

  // Safety violations -- NEVER auto-purged.
  safetyEvents: SafetyEvent[];
  unacknowledgedCount: number;

  // Derived (computed from currentRound for convenience).
  currentPhase: RoundPhase | 'idle';
  currentLeader: number;

  // Aggregate metrics (computed from roundHistory).
  avgFinalizationMs: number;
  nullificationRate: number;

  // Subscription state.
  subscribed: boolean;

  // Actions.
  addEvent: (event: ConsensusEvent) => void;
  addSafetyEvent: (event: SafetyEvent) => void;
  acknowledgeSafetyEvent: (index: number) => void;
  setValidators: (info: {
    validators: ValidatorInfo[];
    count: number;
    threshold: number;
  }) => void;
  updateFromNodeStatus: (status: NodeStatus) => void;
  setSubscribed: (subscribed: boolean) => void;
}

// ---------------------------------------------------------------------------
// Round phase transition logic
// ---------------------------------------------------------------------------

/**
 * Given a ConsensusEvent, compute the next RoundPhase for the current round.
 * Returns null if the event does not trigger a phase change.
 */
function nextPhase(event: ConsensusEvent): RoundPhase | null {
  switch (event.type) {
    case 'notarize':
      return 'notarizing';
    case 'notarization':
      return 'certifying';
    case 'certification':
    case 'finalize':
      return 'finalizing';
    case 'finalization':
      return 'finalized';
    case 'nullify':
      return 'nullifying';
    case 'nullification':
      return 'nullified';
    default:
      // Safety events do not change the round phase.
      return null;
  }
}

/**
 * Apply a timestamp to the appropriate field on a ConsensusRound draft.
 */
function stampRound(round: ConsensusRound, event: ConsensusEvent): void {
  switch (event.type) {
    case 'notarize':
      round.notarizeAt = event.timestampMs;
      break;
    case 'notarization':
      round.notarizationAt = event.timestampMs;
      break;
    case 'certification':
      round.certificationAt = event.timestampMs;
      break;
    case 'finalize':
      round.finalizeAt = event.timestampMs;
      break;
    case 'finalization':
      round.finalizationAt = event.timestampMs;
      break;
    case 'nullify':
      round.nullifyAt = event.timestampMs;
      break;
    case 'nullification':
      round.nullificationAt = event.timestampMs;
      break;
  }
}

/**
 * Create a fresh ConsensusRound for a new view.
 */
function createRound(view: number, validatorCount: number): ConsensusRound {
  return {
    view,
    leader: validatorCount > 0 ? view % validatorCount : 0,
    phase: 'proposing',
    payload: null,
    startedAt: Date.now(),
    notarizeAt: null,
    notarizationAt: null,
    certificationAt: null,
    finalizeAt: null,
    finalizationAt: null,
    nullifyAt: null,
    nullificationAt: null,
  };
}

/**
 * Recompute aggregate metrics from the round history array.
 */
function computeAggregates(history: ConsensusRound[]): {
  avgFinalizationMs: number;
  nullificationRate: number;
} {
  if (history.length === 0) {
    return { avgFinalizationMs: 0, nullificationRate: 0 };
  }

  let finalizationTotal = 0;
  let finalizationCount = 0;
  let nullifiedCount = 0;

  for (const round of history) {
    if (round.phase === 'finalized' && round.finalizationAt !== null) {
      finalizationTotal += round.finalizationAt - round.startedAt;
      finalizationCount++;
    }
    if (round.phase === 'nullified') {
      nullifiedCount++;
    }
  }

  return {
    avgFinalizationMs:
      finalizationCount > 0 ? finalizationTotal / finalizationCount : 0,
    nullificationRate: nullifiedCount / history.length,
  };
}

// ---------------------------------------------------------------------------
// Store creation
// ---------------------------------------------------------------------------

export const useConsensusStore = create<ConsensusState>()(
  devtools(
    immer((set, get) => ({
      // ── Initial state ────────────────────────────────────────────
      currentRound: null,
      roundHistory: [] as ConsensusRound[],

      validators: [] as ValidatorInfo[],
      validatorCount: 0,
      threshold: 0,

      safetyEvents: [] as SafetyEvent[],
      unacknowledgedCount: 0,

      currentPhase: 'idle' as RoundPhase | 'idle',
      currentLeader: 0,

      avgFinalizationMs: 0,
      nullificationRate: 0,

      subscribed: false,

      // ── Actions ──────────────────────────────────────────────────

      addEvent: (event: ConsensusEvent) => {
        set((draft) => {
          const validatorCount = draft.validatorCount;

          // Extract view from the event (not all event types carry it).
          let eventView: number | null = null;
          if ('view' in event) {
            eventView = event.view;
          }

          // If we have no current round, or the event's view is ahead,
          // start a new round.
          if (
            eventView !== null &&
            (draft.currentRound === null ||
              eventView > draft.currentRound.view)
          ) {
            // Archive the previous round if it existed and was past proposing.
            if (
              draft.currentRound !== null &&
              draft.currentRound.phase !== 'proposing'
            ) {
              draft.roundHistory.unshift(draft.currentRound);
              while (draft.roundHistory.length > MAX_ROUND_HISTORY) {
                draft.roundHistory.pop();
              }
            }
            draft.currentRound = createRound(eventView, validatorCount);
          }

          const round = draft.currentRound;
          if (round === null) return;

          // Apply phase transition.
          const phase = nextPhase(event);
          if (phase !== null) {
            round.phase = phase;
            draft.currentPhase = phase;
          }

          // Apply timestamp.
          stampRound(round, event);

          // Set payload if the event carries one.
          if ('payload' in event && typeof event.payload === 'string') {
            round.payload = event.payload;
          }

          // Update leader.
          draft.currentLeader = round.leader;

          // On terminal phases (finalized / nullified), archive the round.
          if (phase === 'finalized' || phase === 'nullified') {
            draft.roundHistory.unshift({ ...round });
            while (draft.roundHistory.length > MAX_ROUND_HISTORY) {
              draft.roundHistory.pop();
            }

            // Recompute aggregate metrics.
            const agg = computeAggregates(draft.roundHistory);
            draft.avgFinalizationMs = agg.avgFinalizationMs;
            draft.nullificationRate = agg.nullificationRate;
          }
        });
      },

      addSafetyEvent: (event: SafetyEvent) => {
        set((draft) => {
          draft.safetyEvents.push(event);
          if (!event.acknowledged) {
            draft.unacknowledgedCount++;
          }
        });
      },

      acknowledgeSafetyEvent: (index: number) => {
        set((draft) => {
          const event = draft.safetyEvents[index];
          if (event && !event.acknowledged) {
            event.acknowledged = true;
            draft.unacknowledgedCount = Math.max(
              0,
              draft.unacknowledgedCount - 1,
            );
          }
        });
      },

      setValidators: (info) => {
        set((draft) => {
          draft.validators = info.validators;
          draft.validatorCount = info.count;
          draft.threshold = info.threshold;
        });
      },

      updateFromNodeStatus: (status: NodeStatus) => {
        set((draft) => {
          // Populate validator info if not already set.
          if (draft.validatorCount === 0) {
            draft.validatorCount = status.validatorCount;
            draft.threshold = status.threshold;
            draft.validators = Array.from(
              { length: status.validatorCount },
              (_, i) => ({
                index: i,
                isLocal: i === status.validatorIndex,
              }),
            );
          }

          // If no WS subscription, use poll data to approximate round state.
          if (!draft.subscribed) {
            if (
              draft.currentRound === null ||
              status.currentView > draft.currentRound.view
            ) {
              draft.currentRound = createRound(
                status.currentView,
                status.validatorCount,
              );
            }
            draft.currentLeader = status.currentLeader;
          }
        });
      },

      setSubscribed: (subscribed: boolean) => {
        set((draft) => {
          draft.subscribed = subscribed;
        });
      },
    })),
    { name: 'consensus-store', enabled: import.meta.env.DEV },
  ),
);

// ---------------------------------------------------------------------------
// Selectors
// ---------------------------------------------------------------------------

export const useCurrentRound = () =>
  useConsensusStore((s) => s.currentRound);

export const useCurrentPhase = () =>
  useConsensusStore((s) => s.currentPhase);

export const useCurrentLeader = () =>
  useConsensusStore((s) => s.currentLeader);

export const useRoundHistory = () =>
  useConsensusStore((s) => s.roundHistory);

export const useValidators = () =>
  useConsensusStore((s) => s.validators);

export const useSafetyEvents = () =>
  useConsensusStore((s) => s.safetyEvents);

export const useUnacknowledgedViolations = () =>
  useConsensusStore((s) => s.unacknowledgedCount);

export const useConsensusMetrics = () =>
  useConsensusStore((s) => ({
    avgFinalizationMs: s.avgFinalizationMs,
    nullificationRate: s.nullificationRate,
  }));
```

### Safety events are never auto-purged

The `safetyEvents` array grows unbounded. This is deliberate. A conflicting
finalization is a once-in-a-lifetime event that must be preserved for forensic
review. The user must explicitly acknowledge each violation to dismiss the alert
UI, but the event remains in the array permanently.

---

## 6. Store Wiring (`src/stores/index.ts`)

Initializes all stores and wires bus events to store actions. This is the single
place where the event bus connects to the Zustand stores.

```typescript
// src/stores/index.ts

import { bus } from '@/bus/events';
import type { BusEvents } from '@/bus/events';
import { useChainStore } from '@/stores/chain';
import { useUIStore } from '@/stores/ui';
import { useConsensusStore } from '@/stores/consensus';
import type { SafetyEvent } from '@/data/consensus';

// ---------------------------------------------------------------------------
// Re-exports for convenience
// ---------------------------------------------------------------------------

export { useChainStore } from '@/stores/chain';
export {
  useLatestBlockNumber,
  useLatestBlock,
  useBlock,
  useBlockOrder,
  useTransaction,
  useReceipt,
  useAddressMeta,
  useFeeHistory,
  useConnectionState,
  useRpcLatency,
} from '@/stores/chain';

export { useUIStore } from '@/stores/ui';
export {
  useActiveScene,
  useSelectedEntity,
  useSearchState,
  useAmbientMode,
  useSidebarCollapsed,
  usePerformanceTier,
} from '@/stores/ui';

export { useConsensusStore } from '@/stores/consensus';
export {
  useCurrentRound,
  useCurrentPhase,
  useCurrentLeader,
  useRoundHistory,
  useValidators,
  useSafetyEvents,
  useUnacknowledgedViolations,
  useConsensusMetrics,
} from '@/stores/consensus';

// ---------------------------------------------------------------------------
// Bus -> Store wiring
//
// Called once at application startup (in main.tsx). Each bus.on() call returns
// nothing (mitt handlers are void). Cleanup is not needed because stores and
// the bus share the application lifetime.
// ---------------------------------------------------------------------------

/** Safety event types that should be recorded as SafetyEvents. */
const SAFETY_TYPES = new Set([
  'conflictingNotarize',
  'conflictingFinalize',
  'nullifyFinalize',
]);

export function initializeStores(): void {
  const chain = useChainStore.getState();
  const ui = useUIStore.getState();
  const consensus = useConsensusStore.getState();

  // ── Chain events ───────────────────────────────────────────────

  bus.on('block:new', (block) => {
    chain.addBlock(block);
  });

  bus.on('block:reorg', ({ oldHead, newHead }) => {
    chain.handleReorg(oldHead, newHead);
  });

  bus.on('tx:new', (tx) => {
    chain.addTransaction(tx);
  });

  bus.on('tx:pending', (tx) => {
    chain.addTransaction(tx);
  });

  bus.on('tx:confirmed', ({ hash, receipt }) => {
    chain.addReceipt(hash, receipt);
  });

  bus.on('connection:state', (state) => {
    chain.setConnectionState(state);
  });

  // ── Consensus events ──────────────────────────────────────────

  bus.on('consensus:event', (event) => {
    consensus.addEvent(event);

    // Intercept safety violations and record them separately.
    if (SAFETY_TYPES.has(event.type)) {
      const safetyEvent: SafetyEvent = {
        type: event.type as SafetyEvent['type'],
        timestampMs: event.timestampMs,
        acknowledged: false,
      };
      consensus.addSafetyEvent(safetyEvent);
    }
  });

  bus.on('node:status', (status) => {
    consensus.updateFromNodeStatus(status);
  });

  // ── Navigation / UI events ────────────────────────────────────

  bus.on('scene:navigate', (scene) => {
    ui.setScene(scene);
  });

  bus.on('entity:select', (entity) => {
    ui.selectEntity(entity);
  });

  bus.on('entity:deselect', () => {
    ui.selectEntity(null);
  });

  bus.on('search:query', (query) => {
    ui.setSearch(query);
  });

  bus.on('search:result', (results) => {
    ui.setSearchResults(results);
  });
}
```

### Calling `initializeStores()` in `main.tsx`

```typescript
// apps/explorer/src/main.tsx

import { initializeStores } from '@/stores';
import { ConnectionManager } from '@/data/connection';

// 1. Wire bus -> stores (idempotent, call once).
initializeStores();

// 2. Start the data layer.
const connection = new ConnectionManager();
connection.connect();

// 3. React render.
// ...
```

---

## 7. Devtools Integration

All three stores use the `devtools` middleware, gated behind `import.meta.env.DEV`:

```typescript
devtools(
  innerMiddleware((set, get) => ({
    // ...store definition...
  })),
  { name: 'store-name', enabled: import.meta.env.DEV },
)
```

In development:
- The [zustand/devtools](https://github.com/pmndrs/zustand#devtools) middleware
  sends actions to the Redux DevTools browser extension.
- Each store appears as a separate instance (`chain-store`, `ui-store`,
  `consensus-store`).
- Actions are named after the method that called `set()` (immer middleware
  preserves this).
- The wildcard `bus.on('*', ...)` handler in `events.ts` logs every bus event to
  the console.

In production:
- `enabled: import.meta.env.DEV` disables devtools entirely -- zero overhead.
- The wildcard bus handler is tree-shaken by Vite because it is wrapped in
  `if (import.meta.env.DEV)`.

---

## 8. Complete Type Dependencies

For reference, the types consumed by these stores come from two files already
defined in the design spec:

### `src/data/types.ts` (from 04-data-flows.md)

```typescript
// Already defined in the design spec. Key types used by stores:

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
  _activityLevel: number;
  _arrivalTime: number;
}

export interface ChainTransaction {
  hash: `0x${string}`;
  from: `0x${string}`;
  to: `0x${string}` | null;
  value: bigint;
  gasPrice: bigint;
  gas: bigint;
  nonce: number;
  input: `0x${string}`;
  blockNumber: bigint;
  blockHash: `0x${string}`;
  transactionIndex: number;
  type: number;
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
  firstSeen: bigint;
  lastSeen: bigint;
  txCount: number;
  position: { x: number; y: number; z: number };
}

export interface FeeDataPoint {
  blockNumber: bigint;
  baseFee: bigint;
  gasUsedRatio: number;
  reward25: bigint;
  reward50: bigint;
  reward75: bigint;
}
```

### `src/data/consensus.ts` (from 08-consensus-data.md)

```typescript
// Already defined in the design spec. Key types used by stores:

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

export interface ConsensusRound {
  view: number;
  leader: number;
  phase: RoundPhase;
  payload: string | null;
  startedAt: number;
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

export interface SafetyEvent {
  type: 'conflictingNotarize' | 'conflictingFinalize' | 'nullifyFinalize';
  timestampMs: number;
  acknowledged: boolean;
}
```

---

## 9. Verification Checklist

- [ ] **Bus emits and receives typed events correctly**
  - Instantiate `bus` and call `bus.emit('block:new', block)` with a valid
    `ChainBlock`. Verify the handler receives the correct typed payload.
  - Attempt `bus.emit('block:new', 'invalid')` -- TypeScript compiler must reject
    this at build time.

- [ ] **Chain store adds blocks and maintains ring buffer at 128**
  - Call `addBlock()` 200 times with incrementing block numbers.
  - Assert `blocks.size === 128` and `blockOrder.length === 128`.
  - Assert the oldest 72 blocks have been evicted.
  - Assert `blockOrder[0]` is the most recent block number (newest-first ordering).

- [ ] **LRU caches evict at capacity**
  - Create `LRUMap<string, number>(3)`.
  - Insert keys A, B, C, D.
  - Assert `has('A') === false` (evicted as oldest).
  - Call `get('B')` to promote it, then insert E.
  - Assert `has('C') === false` (now oldest after B was promoted).
  - Assert `has('B') === true` (promoted, not evicted).

- [ ] **UI store persists scene to localStorage**
  - Call `setScene('constellation')`.
  - Read `localStorage.getItem('kora-explorer-ui')` and parse JSON.
  - Assert the stored object contains `activeScene: 'constellation'`.
  - Assert it does NOT contain `searchQuery`, `selectedEntity`, or other
    ephemeral state.

- [ ] **Consensus store tracks round history at 32**
  - Call `addEvent()` with a finalization event for views 1 through 50.
  - Assert `roundHistory.length === 32`.
  - Assert the oldest rounds (views 1-18) have been evicted.
  - Assert `avgFinalizationMs` and `nullificationRate` are computed from the
    retained 32 rounds.

- [ ] **Zustand selectors prevent unnecessary re-renders (shallow equality)**
  - Render a component that uses `useLatestBlockNumber()`.
  - Call `chain.addTransaction(tx)` (unrelated state change).
  - Assert the component did NOT re-render (latestBlock did not change).
  - Call `chain.addBlock(block)`.
  - Assert the component DID re-render (latestBlock changed).

- [ ] **Bus cleanup on component unmount (no memory leaks)**
  - Mount a component that calls `bus.on('block:new', handler)` in useEffect.
  - Unmount the component.
  - Call `bus.emit('block:new', block)`.
  - Assert `handler` was NOT called (cleanup ran).
  - Verify no retained references via the browser's Memory tab heap snapshot.

- [ ] **Performance tier auto-detection works on mobile Safari**
  - On an iPhone or iPad, verify `detectPerformanceTier()` returns `'mobile'`.
  - On a MacBook with Safari, verify it returns `'full'` or `'standard'`
    (Safari exposes WebGL2 since 16.4 but does not expose `navigator.deviceMemory`
    -- the fallback of 8 GB ensures it does not incorrectly detect `'light'`).
  - On a desktop Chrome with 4 GB RAM, verify it returns `'standard'` or `'light'`
    depending on GPU renderer string.

---

## Appendix: Middleware Stack

Each store composes middleware in a specific order. The outermost middleware runs
first on writes and last on reads.

### Chain store

```
devtools            -- outermost: sends actions to Redux DevTools
  immer             -- enables mutable draft syntax in set()
    (store logic)   -- innermost: the actual state + actions
```

### UI store

```
devtools            -- outermost: sends actions to Redux DevTools
  persist           -- reads from / writes to localStorage
    (store logic)   -- innermost: the actual state + actions
```

### Consensus store

```
devtools            -- outermost: sends actions to Redux DevTools
  immer             -- enables mutable draft syntax in set()
    (store logic)   -- innermost: the actual state + actions
```

The UI store does not use `immer` because its state is flat primitives and small
objects -- there is no deep nesting that benefits from draft syntax. The chain and
consensus stores use `immer` because they mutate Maps, arrays, and nested round
objects.
