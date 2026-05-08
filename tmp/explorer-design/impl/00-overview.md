# Kora Explorer — Implementation Master Index

The implementation plan for a cinematic block explorer that renders the kora blockchain as a physical, atmospheric space. Built with the ROSEDUST design language on React 19 + Three.js + viem.

Design documents: [`../00-overview.md`](../00-overview.md) through [`../08-consensus-data.md`](../08-consensus-data.md).

---

## Architecture Decisions

| Decision | Choice | Rationale |
|----------|--------|-----------|
| Framework | React 19 + Vite 6 + TypeScript (strict) | Concurrent features for streaming data, wide ecosystem |
| 3D Engine | Three.js via React Three Fiber (R3F) | Hash Terrain (Scene 1), Block Waterfall (Scene 3) |
| 2D / Particles | regl (WebGL2) | GPU particle system for Constellation (Scene 2) -- R3F too heavy for 16K particles |
| State | Zustand v5 (3 stores: chain, ui, consensus) | Minimal, selector-based re-renders, no Redux boilerplate |
| Event Bus | mitt | 200 bytes, fully typed, bridges data layer to scenes without React re-renders |
| RPC Client | viem v2 with custom kora chain definition | Type-safe encoding/decoding, built-in retry/reconnect, HTTP + future WS |
| Routing | React Router 7 | URL-driven detail views, shareable links, browser back/forward |
| Design System | ROSEDUST -- CSS custom properties, no component library | Terminal-existentialist aesthetic, glass panels, atmospheric layers |
| Location | `apps/explorer/` inside the daeji monorepo | Colocated with Rust workspace, no interference with Cargo |

---

## Current RPC State

The kora node today exposes HTTP JSON-RPC only. No WebSocket transport, no subscriptions.

**Available methods:**

| Category | Methods |
|----------|---------|
| Block queries | `eth_blockNumber`, `eth_getBlockByNumber`, `eth_getBlockByHash` |
| State reads | `eth_getBalance`, `eth_getCode`, `eth_getStorageAt` |
| Transaction lifecycle | `eth_sendRawTransaction`, `eth_getTransactionByHash`, `eth_getTransactionReceipt` |
| Execution simulation | `eth_call`, `eth_estimateGas` |
| Fee data | `eth_gasPrice`, `eth_feeHistory` |
| Logs | `eth_getLogs` (block range filters) |
| Kora-specific | `kora_nodeStatus` (chainId, validatorIndex, currentView, finalizedCount, proposedCount, nullifiedCount, peerCount, isLeader) |
| HDC | `hdc_*` (hammingDistance, similarity, bind, bundle, search, vectorId, encode) |

**Not available:**

- `eth_subscribe` / WebSocket transport (jsonrpsee has `ws_addr` configured but not wired)
- `eth_getBlockReceipts` (batch receipt fetch)
- `debug_traceTransaction` / `debug_traceBlockByNumber`
- `kora_consensusState` (rich consensus snapshot)
- `kora_subscribe` (consensus / mempool streaming)
- Filter API (`eth_newBlockFilter`, `eth_getFilterChanges`)

**Consequence for the explorer:** Phase 1 operates entirely on polling. `eth_blockNumber` at 1/s, `eth_feeHistory` at 1/5s. On-demand fetches for block detail, transaction detail, address state. Up to 1s latency on block arrival. No mempool visibility. Consensus data limited to `kora_nodeStatus` counters.

---

## Implementation Plan Documents

| # | Filename | Description | Depends On | Phase |
|---|----------|-------------|------------|-------|
| 01 | [`01-project-scaffold.md`](./01-project-scaffold.md) | Vite project setup, directory structure, tsconfig, package.json, vite.config.ts, monorepo integration, Justfile commands | -- | 0 |
| 02 | [`02-rosedust-tokens.md`](./02-rosedust-tokens.md) | CSS custom properties, @font-face loading, glass panel styles, atmospheric layers (grain, vignette, scanlines, rose wash), reset.css | -- | 0 |
| 03 | [`03-rpc-client.md`](./03-rpc-client.md) | viem client with custom kora chain definition, ConnectionManager, block poller, fee history poller, optimistic skip for empty blocks, error handling, reconnection with exponential backoff, TypeScript type definitions for all RPC responses | -- | 0 |
| 04 | [`04-state-stores.md`](./04-state-stores.md) | Zustand stores (chain, ui, consensus), mitt event bus with BusEvents type map, LRU block cache, address set, fee history ring buffer, wiring: bus events into store actions, scene-vs-panel subscription strategy | 03 | 1 |
| 05 | [`05-scene-terrain.md`](./05-scene-terrain.md) | Scene 1: Block terrain heightmap. Hash-to-heightmap algorithm (32 bytes to 16x16 grid), vertex/fragment shaders (height-based color lerp + exponential fog), ring buffer of 200 tiles, frustum culling, camera tracking, transaction light beams, R3F Canvas integration | 02, 04 | 2 |
| 06 | [`06-scene-constellation.md`](./06-scene-constellation.md) | Scene 2: Transaction constellation. GPU particle system via regl, transform feedback for physics, address-to-position algorithm (golden-ratio spiral), transaction arc beziers, node rendering (dormant/active/contract), idle animation, pending tx lifecycle (Phase 2 only) | 02, 04 | 2 |
| 07 | [`07-scene-waterfall.md`](./07-scene-waterfall.md) | Scene 3: Live block waterfall + gas waveform. Falling block cards (Canvas2D), hash-derived barcode fill, stack compression, gas waveform from eth_feeHistory, block pulse heartbeat indicator | 02, 04 | 2 |
| 08 | [`08-scene-consensus.md`](./08-scene-consensus.md) | Scene 4: Consensus ring visualization. Validator nodes on ring, threshold arc, center block crystallization, round phase animation (propose through finalize/nullify), safety violation rendering, round history timeline, fallback to block-arrival timing when kora_subscribe unavailable | 02, 04 | 2 |
| 09 | [`09-panels-search.md`](./09-panels-search.md) | Glass panels (GlassPanel wrapper, StatusPanel, BlockInfoPanel, FeeWaveform), SearchOverlay (local-first with RPC fallback, keyboard nav), detail views (BlockDetail with hash art, TxDetail with flow diagram, AddressDetail with constellation zoom) | 02, 04 | 2 |
| 10 | [`10-interaction-routing.md`](./10-interaction-routing.md) | React Router 7 route definitions, URL mapping for all detail views, keyboard shortcuts (1/2/3 mode switch, Space pause, / search, Esc back), ambient mode (30s idle timeout, panel opacity fade), touch gestures, accessibility (focus ring, aria labels, reduced motion, screen reader), SceneManager crossfade transitions | 04, 05-09 | 3 |
| 11 | [`11-rpc-backend-additions.md`](./11-rpc-backend-additions.md) | Rust backend: WebSocket transport wiring in jsonrpsee, eth_subscribe (newHeads, logs, newPendingTransactions), SubscriptionManager with broadcast channels, eth_getBlockReceipts, kora_consensusState (rich snapshot), kora_subscribe (consensus, mempool), ConsensusReporter impl | -- | Backend |
| 12 | [`12-testing-deployment.md`](./12-testing-deployment.md) | Vitest unit tests (hash algorithms, LRU cache, color derivation, format utils), Vitest + React Testing Library component tests, MSW mock server for RPC, Playwright visual regression (deterministic block hashes for screenshots), static build optimization, Docker compose service, nginx reverse proxy config | all | 4 |

---

## Critical Path

```
Phase 0 (parallel, no dependencies):

    01-project-scaffold ─────┐
    02-rosedust-tokens  ─────┤
    03-rpc-client       ─────┤
                              │
Phase 1:                      ▼
    04-state-stores     ──── (needs 03)
                              │
Phase 2 (parallel):           ▼
    05-scene-terrain    ─────┐ (needs 02, 04)
    06-scene-constellation ──┤ (needs 02, 04)
    07-scene-waterfall  ─────┤ (needs 02, 04)
    08-scene-consensus  ─────┤ (needs 02, 04)
    09-panels-search    ─────┤ (needs 02, 04)
                              │
Phase 3:                      ▼
    10-interaction-routing ── (needs 04, 05-09)

Phase Backend (any time, independent):
    11-rpc-backend-additions

Phase 4 (after everything):
    12-testing-deployment ─── (needs all)
```

Phase 0 items are fully parallel. Phase 2 items are fully parallel once Phase 1 completes. Phase Backend can happen at any point -- it unlocks WebSocket subscriptions and richer consensus data but the explorer functions without it via polling.

---

## Repository Structure

```
apps/explorer/
├── index.html
├── package.json
├── tsconfig.json
├── tsconfig.node.json
├── vite.config.ts
├── .env                        # VITE_RPC_HTTP, VITE_RPC_WS
├── public/
│   └── fonts/
│       ├── Fraunces-Italic-Variable.woff2
│       └── JetBrainsMono-Variable.woff2
├── src/
│   ├── main.tsx                 # entry point, bus->store wiring, ConnectionManager init
│   ├── App.tsx                  # root layout, atmospheric layers, scene/panel composition
│   ├── routes.tsx               # React Router 7 route definitions
│   ├── styles/
│   │   ├── rosedust.css         # ROSEDUST design tokens (:root custom properties)
│   │   ├── glass.module.css     # glass panel component styles
│   │   ├── atmosphere.module.css # grain, vignette, scanlines, rose wash
│   │   └── reset.css            # minimal CSS reset
│   ├── rpc/
│   │   ├── client.ts            # viem public client + custom kora chain definition
│   │   ├── connection.ts        # ConnectionManager (poller + optional WS subscriber)
│   │   ├── types.ts             # ChainBlock, ChainTransaction, ChainReceipt, ChainLog, etc.
│   │   └── polling.ts           # block poller (1/s), fee history poller (1/5s)
│   ├── stores/
│   │   ├── chain.ts             # useChainStore: blocks (LRU 500), txs, addresses, feeHistory
│   │   ├── ui.ts                # useUIStore: activeScene, selectedEntity, search, ambient
│   │   └── consensus.ts         # useConsensusStore: rounds, votes, safetyEvents, metrics
│   ├── bus/
│   │   └── events.ts            # mitt event bus instance + BusEvents type map
│   ├── scenes/
│   │   ├── terrain/             # Scene 1: Hash Terrain (R3F)
│   │   │   ├── TerrainScene.tsx
│   │   │   ├── TerrainMesh.tsx
│   │   │   ├── terrain.vert.glsl
│   │   │   ├── terrain.frag.glsl
│   │   │   ├── hashmap.ts       # block hash -> 16x16 heightmap
│   │   │   └── beams.tsx        # transaction light beams
│   │   ├── constellation/       # Scene 2: Particle Constellation (regl)
│   │   │   ├── ConstellationCanvas.tsx
│   │   │   ├── particles.ts     # GPU particle buffer + transform feedback
│   │   │   ├── nodes.ts         # address node placement + rendering
│   │   │   ├── arcs.ts          # transaction arc bezier paths
│   │   │   └── constellation.frag.glsl
│   │   ├── waterfall/           # Scene 3: Block Waterfall (Canvas2D)
│   │   │   ├── WaterfallScene.tsx
│   │   │   ├── BlockCard.tsx
│   │   │   ├── hashbar.ts       # hash-derived barcode pattern
│   │   │   └── pulse.tsx        # block pulse heartbeat
│   │   └── consensus/           # Scene 4: Consensus Ring
│   │       ├── ConsensusRing.tsx
│   │       ├── ValidatorNode.tsx
│   │       ├── ThresholdArc.tsx
│   │       └── RoundHistory.tsx
│   ├── panels/
│   │   ├── GlassPanel.tsx       # reusable glass panel wrapper
│   │   ├── StatusBar.tsx        # connection LED, chain id, block count
│   │   ├── BlockDetail.tsx      # single block view with hash art
│   │   ├── TxDetail.tsx         # transaction flow diagram + execution data
│   │   ├── AddressDetail.tsx    # identity, balance, constellation zoom
│   │   ├── SearchOverlay.tsx    # / to open, local-first search, keyboard nav
│   │   └── FeeWaveform.tsx      # sparkline from eth_feeHistory
│   ├── hooks/
│   │   ├── useBlockPoller.ts    # starts/stops block polling, handles backfill
│   │   ├── useAmbientMode.ts    # 30s idle -> panel fade, interaction -> restore
│   │   └── useFrameMonitor.ts   # dev-only performance overlay
│   └── lib/
│       ├── lru.ts               # LRU cache (O(1) get/set/delete via Map ordering)
│       ├── hashArt.ts           # block hash -> generative art (8 pattern algorithms)
│       └── format.ts            # address truncation, ETH formatting, hex conversions
└── tests/
    ├── unit/                    # vitest: hash algorithms, LRU, format, color
    ├── integration/             # vitest + MSW: RPC response handling, reconnection
    └── e2e/                     # playwright: visual regression with deterministic data
```

---

## Data Flow Architecture

```
                ┌─────────────────────────────┐
                │    kora RPC (HTTP only)      │
                │    65.109.61.210:8545        │
                └──────────────┬──────────────┘
                               │
                ┌──────────────▼──────────────┐
                │    ConnectionManager         │
                │                              │
                │  Phase 1: Poller             │
                │    eth_blockNumber   1/s     │
                │    eth_feeHistory    1/5s    │
                │    on-demand: block, tx,     │
                │      address, logs           │
                │                              │
                │  Phase 2 (after doc 11):     │
                │    WebSocket subscriber      │
                │    newHeads, logs,            │
                │    pendingTransactions,       │
                │    consensus, mempool         │
                └──────────────┬──────────────┘
                               │
                ┌──────────────▼──────────────┐
                │    EventBus (mitt)            │
                │                              │
                │  block:new      -> ChainBlock │
                │  tx:confirmed   -> ChainTx    │
                │  tx:pending     -> PendingTx  │  (Phase 2)
                │  fee:update     -> FeeData[]  │
                │  address:seen   -> 0x...      │
                │  log:new        -> ChainLog   │
                │  consensus:event -> Event     │  (Phase 2)
                │  consensus:round -> Round     │  (Phase 2)
                │  consensus:safety -> Safety   │  (Phase 2)
                │  connection:status            │
                └──────────┬──────────────┬────┘
                           │              │
              ┌────────────▼──┐     ┌─────▼────────────┐
              │  Zustand Stores│     │  Scenes (direct)  │
              │  (React path) │     │  (imperative path) │
              │               │     │                    │
              │  chain store  │     │  terrain.addBlock() │
              │  ui store     │     │  particles.spawn()  │
              │  consensus    │     │  waterfall.drop()   │
              │  store        │     │  ring.advance()     │
              └───────┬───────┘     └────────────────────┘
                      │
              ┌───────▼───────┐
              │  React Panels  │
              │  (re-render on │
              │  selector      │
              │  change only)  │
              └────────────────┘
```

Scenes subscribe to the EventBus directly and update imperatively -- no React re-renders per block for 60fps animation. Panels subscribe to Zustand stores via selectors and re-render only when their specific slice changes.

---

## Performance Targets

| Metric | Budget |
|--------|--------|
| Frame budget (60fps) | 16.6ms: scene render 6ms, particle update 4ms, React 2ms, GC 1ms, headroom 3.6ms |
| Initial load (LCP) | < 2.5s (hero scene visible, fonts may still swap) |
| Core JS bundle (gzip) | ~80KB (React, Zustand, router, panels, data layer) |
| Three.js chunk (gzip) | ~120KB (lazy, loaded on first Terrain render) |
| Constellation chunk (gzip) | ~60KB (lazy, loaded on first Constellation render) |
| JS heap (steady state) | < 20MB (LRU bounds block cache, ring buffer bounds fee history) |
| GPU memory (all scenes) | < 100MB (terrain ring buffer + particle buffers + textures) |
| Total memory | < 150MB |
| Network (steady state) | 5-75 KB/s (polling), lower with WS subscriptions |

### Progressive Enhancement

| Tier | Hardware | Scenes | FPS |
|------|----------|--------|-----|
| Full | Desktop, dGPU, 8GB+ RAM | All scenes, atmosphere, audio | 60 |
| Standard | Desktop/laptop, iGPU, 4GB+ | Terrain + waterfall, 8K particles, atmosphere | 60 |
| Light | Low-end laptop, 2GB | Mosaic only (Canvas2D), no 3D | 30 |
| Mobile | Phone/tablet | Mosaic only, simplified panels, no atmosphere | 30 |

Detection runs once at startup. `navigator.deviceMemory`, WebGL2 availability, and GPU renderer string determine the tier.

---

## Key Design System References

**ROSEDUST tokens** (complete set in design doc [`../02-visual-architecture.md`](../02-visual-architecture.md)):

| Token | Value | Usage |
|-------|-------|-------|
| `--rd-void` | #060608 | All backgrounds |
| `--rd-rose` | #aa7088 | Primary accent: blocks, heartbeat |
| `--rd-bone-bright` | #d8c8a0 | Value, transactions, warmth |
| `--rd-dream` | #7a7a98 | Pending, unresolved states |
| `--rd-danger` | #cc5555 | Failed/reverted, safety violations |
| `--rd-text` | #c8b8c0 | Default text (9.4:1 contrast on void) |
| `--rd-text-ghost` | #3a303a | Decorative only, fails contrast |
| `--rd-glass-bg` | rgba(8, 8, 12, 0.45) | Panel backgrounds |
| `--rd-glass-blur` | 12px | Backdrop blur |
| `--rd-font-mono` | JetBrains Mono | All data, labels, code, addresses |
| `--rd-font-display` | Fraunces (italic) | Block numbers, hero metrics |

**Atmospheric layers** (applied globally, all modes):

1. Grain (z-index 9997): fractal noise at 3.5% opacity, overlay blend
2. Vignette (z-index 9998): radial gradient darkening edges
3. Scanlines (z-index 9999): 2px/1px repeating gradient at 6% opacity
4. Rose wash (z-index 99): radial gradient driven by `--rd-activity` (0 to 0.15), pulsing with chain activity

**Panel rules:** sharp corners always, glass bg, 1px border at 7% white, backdrop-filter blur(12px), left rose accent on active panels, LED dot for streaming state, hover translateY(-2px) with asymmetric timing (80ms in, 120ms out).

---

## Five Most Impactful Items to Build First

**1. Project scaffold + ROSEDUST tokens (docs 01 + 02)**

Everything else depends on having a working Vite project with the design system loaded. This is the foundation: `npm run dev` working, fonts loaded, glass panels rendering on a void background, atmospheric layers visible. Combine these two because they have no dependencies and the design system must be testable immediately. Without this, nothing else can be started or visually validated.

**2. RPC client + block poller (doc 03)**

The explorer is useless without chain data. The viem client, custom kora chain definition, and polling loop are the oxygen supply. Once this works, `eth_blockNumber` fires every second, blocks are fetched with full transaction bodies when present, and the EventBus starts emitting `block:new` events. This is the first moment the explorer becomes alive -- data flows into the system. The ConnectionManager also handles error recovery and reconnection, which prevents the explorer from going permanently dark on a transient RPC failure.

**3. State stores + event bus wiring (doc 04)**

The stores are the connective tissue. The chain store holds the LRU block cache and address set. The UI store tracks which scene is active, what entity is selected, and ambient mode state. The consensus store (initially sparse) holds round metrics from `kora_nodeStatus`. The event bus wiring -- `bus.on('block:new', ...)` feeding `useChainStore.getState().addBlock()` -- is what makes the data layer and the UI layer speak to each other. This also establishes the critical architectural boundary: scenes read from the bus directly (imperative, no re-renders), panels read from Zustand (declarative, React re-renders via selectors).

**4. Block waterfall scene (doc 07)**

Of the four scenes, the waterfall is the fastest to get visually impressive. It uses Canvas2D (no Three.js chunk needed), renders 50 block cards with hash-derived barcode fills, and animates block arrivals with CSS transforms. It proves the full data pipeline end-to-end: RPC poll discovers new block, poller emits `block:new`, waterfall scene catches the event, renders a falling card. The gas waveform from `eth_feeHistory` adds a second visual channel. This is the first scene that makes the explorer feel real.

**5. Glass panels + status bar (doc 09, partial)**

The StatusBar (connection LED, chain ID, block number, gas sparkline) and GlassPanel wrapper give the explorer its identity. Without panels, the scenes are just visualizations with no context. The StatusBar provides the minimum dashboard: is the explorer connected, what chain, what block number, how fast are blocks arriving. The GlassPanel component is reused by every other panel (BlockDetail, TxDetail, AddressDetail, SearchOverlay), so building it early means all subsequent panel work is composition on a proven base.

After these five, the Terrain scene (doc 05) and Constellation scene (doc 06) are the natural next targets -- they deliver the signature visuals but require the Three.js and regl chunks respectively, so they are heavier to implement and test.
