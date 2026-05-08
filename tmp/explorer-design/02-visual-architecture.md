# Visual Architecture

The explorer is three things layered on top of each other:
a **scene** (3D/WebGL), a **dashboard** (glass panels), and an **atmosphere** (grain, scanlines, vignette).

## Design Language: ROSEDUST Applied to Chain Data

Every visual decision maps from the ROSEDUST system. The explorer is not a separate design —
it is the same terminal-existentialist aesthetic rendering a new domain.

### Color Mapping

| Chain concept | ROSEDUST color | Rationale |
|---------------|----------------|-----------|
| Blocks (confirmed) | `--rose` / `--rose-glow` (#aa7088 / #dca5bd) | Blocks are the heartbeat, rose is the life signal |
| Transactions (confirmed) | `--bone-bright` (#d8c8a0) | Transactions carry value — bone = value |
| Pending transactions | `--dream` / `--dream-bright` (#7a7a98 / #9494b4) | Not yet real — dream state |
| Gas / fees | `--warning` (#c89a68) | Burnt amber = combustion, cost |
| Contract creation | `--rose-bright` (#cc90a8) | Birth = bright rose |
| Failed / reverted | `--danger` (#cc5555) | Coral red, used sparingly |
| State roots | `--text-dim` (#6a5a68) | Structural, not featured |
| Validator activity | `--success` (#7a8a78) | Sage green = healthy participation |
| Empty blocks | `--text-ghost` (#3a303a) | Nearly invisible — nothing happened |

### Typography Mapping

| Element | Font | Size | Style |
|---------|------|------|-------|
| Block number (hero) | Fraunces italic | 38px | bone-bright, metric style |
| Block hash | JetBrains Mono | 11px | text-dim, tracking 0.06em |
| Transaction value | JetBrains Mono | 14px | bone-bright |
| Address labels | JetBrains Mono | 10px | uppercase, text-dim, tracking 0.28em |
| Section tags | JetBrains Mono | 11px | `—— 01 · BLOCKS`, tracking 0.32em |
| Status text | JetBrains Mono | 10px | uppercase, LED dot prefix |

### No Border Radius

All panels, cards, buttons: sharp corners. Rectangles. The chain is precise, not friendly.

---

## Layout Modes

The explorer has three modes. User toggles between them.
Each mode foregrounds different data at different scales.

### Mode 1: TERRAIN (Default — Immersive)

Full-viewport Three.js scene. Chain data rendered spatially.
Dashboard elements float as glass overlays at screen edges.

```
┌──────────────────────────────────────────────────────────────┐
│                                                              │
│  ┌─ KORA ──────────────────────┐     ┌─ BLOCK 286,401 ────┐ │
│  │  ● CONNECTED  1337          │     │  0x6d1f...ac1c     │ │
│  │  ▁▂▃▅▇▅▃▁ gas              │     │  0 txns  0 gas     │ │
│  └─────────────────────────────┘     │  1.00s ago         │ │
│                                      └────────────────────┘ │
│                                                              │
│              ╱╲                                              │
│          ╱╲╱  ╲╱╲        ← hash terrain, growing rightward  │
│      ╱╲╱        ╲╱╲                                          │
│  ╱╲╱              ╲╱╲                                        │
│                        ╲                                     │
│                                                              │
│    ◌ ─────── ● ─────── ◌      ← transaction particle arcs   │
│   0xa3..    value      0xb7..                                │
│                                                              │
│                                                              │
│  ┌─ FEE HISTORY ──────────────────────────────────────────┐  │
│  │ ▁▁▁▁▁▂▁▁▁▁▁▁▁▁▁▁▂▁▁▁▁▁▁ base fee (1 gwei flat)      │  │
│  └────────────────────────────────────────────────────────┘  │
└──────────────────────────────────────────────────────────────┘
```

**Scene occupies 100vh.** Glass panels are `position: fixed`, `pointer-events: auto` on the panels only.
Background: the void (#060608). Scene renders on a transparent-alpha canvas.

### Mode 2: MOSAIC (Data-Dense)

Grid layout. Each block is a tile. Click to expand. No 3D — pure 2D generative art + data panels.

```
┌──────────────────────────────────────────────────────────────┐
│  —— 01 · BLOCK MOSAIC                          KORA / 1337  │
│                                                              │
│  ┌────┬────┬────┬────┬────┬────┬────┬────┬────┬────┬────┐   │
│  │▓▓▓▓│░░░░│▒▒▒▒│████│░░░░│▓▓▓▓│▒▒▒▒│░░░░│████│▓▓▓▓│▒▒▒▒│  │
│  │▓▓▓▓│░░░░│▒▒▒▒│████│░░░░│▓▓▓▓│▒▒▒▒│░░░░│████│▓▓▓▓│▒▒▒▒│  │
│  │ 91 │ 92 │ 93 │ 94 │ 95 │ 96 │ 97 │ 98 │ 99 │ 00 │ 01 │  │
│  └────┴────┴────┴────┴────┴────┴────┴────┴────┴────┴────┘   │
│  ┌────┬────┬────┬────┬────┬────┬────┬────┬────┬────┬────┐   │
│  │    │    │    │    │    │    │    │    │    │    │    │  │
│  │    │    │    │    │    │    │    │    │    │    │    │  │
│  │ 80 │ 81 │ 82 │ 83 │ 84 │ 85 │ 86 │ 87 │ 88 │ 89 │ 90 │  │
│  └────┴────┴────┴────┴────┴────┴────┴────┴────┴────┴────┘   │
│                                                              │
│  ┌─ BLOCK 286,394 ─────────────────────────────────────────┐ │
│  │  HASH      0x03c3ced73420b86f54501198836e6b...          │ │
│  │  STATE     0x3eef323af7ecaed386b057ac7fba53...          │ │
│  │  GAS       0 / 250,000,000                              │ │
│  │  TXNS      0                                            │ │
│  └─────────────────────────────────────────────────────────┘ │
└──────────────────────────────────────────────────────────────┘
```

**Tile generation:** Each block hash → deterministic visual pattern.
The 32 bytes of the hash seed: hue rotation, pattern density, symmetry axis, fill algorithm.
Empty blocks are ghost-opacity. Blocks with transactions are vivid.
The grid scrolls — new blocks append at top-right, old blocks flow down-left.

### Mode 3: DETAIL (Single Entity Focus)

Full-page view of one block, one transaction, or one address.
Split layout: data on left, visualization on right.

```
┌──────────────────────────────────────────────────────────────┐
│  ← BACK                                    BLOCK 286,401    │
│                                                              │
│  ┌─ DATA ────────────────────┐  ┌─ VISUAL ────────────────┐ │
│  │                           │  │                          │ │
│  │  HASH                     │  │   ┌──────────────────┐   │ │
│  │  0x6d1fb175dce4b6795c...  │  │   │  generative art  │   │ │
│  │                           │  │   │  from this block  │   │ │
│  │  PARENT                   │  │   │  hash — unique    │   │ │
│  │  0x469162b1b3e2cc0b52...  │  │   │  to this block   │   │ │
│  │                           │  │   └──────────────────┘   │ │
│  │  STATE ROOT               │  │                          │ │
│  │  0x3eef323af7ecaed386...  │  │   Transaction flow       │ │
│  │                           │  │   diagram (if any txns)  │ │
│  │  TRANSACTIONS    0        │  │                          │ │
│  │  GAS USED        0        │  │   or                     │ │
│  │  GAS LIMIT       250M     │  │                          │ │
│  │  BASE FEE        1 gwei   │  │   State diff tree        │ │
│  │  TIMESTAMP       286401   │  │   (if debug_trace avail) │ │
│  │                           │  │                          │ │
│  └───────────────────────────┘  └──────────────────────────┘ │
└──────────────────────────────────────────────────────────────┘
```

---

## Atmospheric Layers

Applied globally across all modes. These are what make it feel *crafted* instead of *built*.

### Layer 1: Grain (z-index 9997)
```css
.grain {
  position: fixed; inset: 0; pointer-events: none; z-index: 9997;
  opacity: 0.035; mix-blend-mode: overlay;
  background-image: url("data:image/svg+xml;utf8,<svg ...>
    <feTurbulence type='fractalNoise' baseFrequency='0.85' numOctaves='3' stitchTiles='stitch'/>
  </svg>");
}
```
Fractal noise at 3.5% opacity. Gives every surface subtle texture. Never clean digital.

### Layer 2: Vignette (z-index 9998)
```css
body::before {
  background: radial-gradient(ellipse at 50% 30%, transparent 50%, rgba(6,6,8,0.72) 100%);
}
```
Directional darkening. Edges recede. Center focuses. Cinematic framing.

### Layer 3: Scanlines (z-index 9999)
```css
body::after {
  background: repeating-linear-gradient(to bottom,
    transparent 0, transparent 2px,
    rgba(0,0,0,0.45) 2px, rgba(0,0,0,0.45) 3px);
  opacity: 0.06; mix-blend-mode: multiply;
}
```
CRT artifact. 2px transparent, 1px dark, repeating. At 6% opacity it's subliminal — you feel it more than see it.

### Layer 4: Rose Wash (unique to explorer)
A subtle `radial-gradient` that tracks the most recent block's activity level.
Empty blocks → wash is invisible. Full blocks → faint rose light emanates from the center.
```css
.rose-wash {
  background: radial-gradient(ellipse at 50% 50%,
    rgba(170, 112, 136, var(--activity)) 0%,
    transparent 60%);
  transition: --activity 2s ease-out;
}
```
The room *breathes* with chain activity.

---

## Glass Panel System

All data overlays use the same glass component:

```
┌─ LABEL ──────────────────────────┐
│                                  │  ← inset 0 1px 0 rgba(255,255,255,0.06)
│   content                        │  ← backdrop-filter: blur(12px) saturate(180%)
│                                  │  ← background: rgba(8, 8, 12, 0.45)
│                                  │  ← border: 1px solid rgba(255,255,255,0.07)
└──────────────────────────────────┘
```

- Left rose accent border (2px `--rose-dim`) on panels showing active/live data
- LED status dot (5px, pulsing) in panel headers for connected/streaming states
- Mono 10px uppercase label with tracking 0.28em
- Sharp corners. Always.

---

## Responsive Behavior

| Breakpoint | Adaptation |
|------------|------------|
| > 1400px | Full layout, terrain mode default, side panels |
| 1100-1400px | Panels stack below scene, scene height reduced |
| 760-1100px | Mosaic default (no 3D), panels full-width |
| < 760px | Single-column, mosaic tiles 3-wide, simplified panels |

Mobile does not attempt 3D. It defaults to MOSAIC mode with hash-art tiles, which is visually striking on its own and doesn't drain batteries.

---

## Tech Stack

| Layer | Choice | Rationale |
|-------|--------|-----------|
| Framework | React 19 + Vite 6 | Largest ecosystem, wide contributor base, concurrent features for streaming data |
| Language | TypeScript (strict) | Type safety for RPC response shapes and WebGL buffer layouts |
| 3D Engine | Three.js r170+ | Hash Terrain and Block Waterfall scenes. Managed via `@react-three/fiber` for React integration |
| 2D/Particle | Raw WebGL2 via `regl` | Particle Constellation needs thousands of GPU-driven particles — R3F is too heavy here |
| Styling | CSS Modules + CSS custom properties | No runtime CSS-in-JS. ROSEDUST tokens as custom properties on `:root`. Modules for component scoping |
| State | Zustand | Minimal, non-opinionated. Single store for chain state, separate store for UI state. No Redux boilerplate |
| RPC Client | viem | Type-safe Ethereum RPC client with WebSocket transport. Handles encoding/decoding, retry, reconnect |
| Routing | React Router 7 | URL-driven navigation for block/tx/address detail views. Shareable links |
| Animation | Framer Motion (panels) + GSAP (scenes) | Framer for React component transitions, GSAP for WebGL-coordinated timeline animations |
| Audio | Tone.js | Web Audio API wrapper with oscillators and filters for chain sonification |
| Build | Vite 6 | Fast HMR, optimized production builds, WebGL shader imports via vite-plugin-glsl |

### Project Structure

```
apps/explorer/
├── index.html
├── package.json
├── tsconfig.json
├── vite.config.ts
├── public/
│   └── fonts/
│       ├── Fraunces-Italic-Variable.woff2
│       └── JetBrainsMono-Variable.woff2
├── src/
│   ├── main.tsx                    # Entry point, providers
│   ├── App.tsx                     # Root layout, mode switching
│   ├── router.tsx                  # Route definitions
│   │
│   ├── design/                     # ROSEDUST implementation
│   │   ├── tokens.css              # CSS custom properties (:root)
│   │   ├── reset.css               # Minimal reset
│   │   ├── typography.css          # @font-face, type scale
│   │   ├── atmosphere.css          # Grain, vignette, scanlines, rose-wash
│   │   └── glass.module.css        # Glass panel component styles
│   │
│   ├── data/                       # Chain data layer
│   │   ├── rpc.ts                  # viem client setup (HTTP + WS)
│   │   ├── poller.ts               # Phase 1: polling loop
│   │   ├── subscriber.ts           # Phase 2: WS subscription manager
│   │   ├── bus.ts                  # EventBus (mitt-based)
│   │   ├── cache.ts                # LRU block cache, address set, fee ring buffer
│   │   ├── store.ts                # Zustand chain state store
│   │   └── types.ts                # Shared TypeScript interfaces
│   │
│   ├── scenes/                     # WebGL/Canvas visualizations
│   │   ├── terrain/                # Scene 1: Hash Terrain
│   │   │   ├── TerrainScene.tsx    # R3F Canvas wrapper
│   │   │   ├── TerrainMesh.tsx     # Tile geometry + shader material
│   │   │   ├── terrain.vert.glsl   # Vertex shader (height + fog)
│   │   │   ├── terrain.frag.glsl   # Fragment shader (color lerp + fog)
│   │   │   ├── hashmap.ts          # Block hash → heightmap conversion
│   │   │   └── beams.tsx           # Transaction light beams
│   │   │
│   │   ├── constellation/          # Scene 2: Particle Constellation
│   │   │   ├── ConstellationCanvas.tsx
│   │   │   ├── particles.ts        # regl-based GPU particle system
│   │   │   ├── nodes.ts            # Address node placement + rendering
│   │   │   ├── arcs.ts             # Transaction arc bezier paths
│   │   │   └── constellation.frag.glsl
│   │   │
│   │   ├── waterfall/              # Scene 3: Block Waterfall
│   │   │   ├── WaterfallScene.tsx
│   │   │   ├── BlockCard.tsx       # Individual falling block
│   │   │   ├── hashbar.ts          # Hash-derived barcode pattern
│   │   │   └── pulse.tsx           # Block pulse heartbeat indicator
│   │   │
│   │   ├── consensus/              # Scene 4: Consensus Ring
│   │   ├── aurora/                 # Scene 5: State Diff Aurora
│   │   ├── organism/               # Scene 6: Contract Organism
│   │   └── SceneManager.tsx        # Scene composition + crossfade
│   │
│   ├── panels/                     # Glass panel UI components
│   │   ├── GlassPanel.tsx          # Reusable glass panel wrapper
│   │   ├── StatusPanel.tsx         # Connection status + chain info
│   │   ├── BlockInfoPanel.tsx      # Latest block details
│   │   ├── FeeWaveform.tsx         # Fee history sparkline
│   │   ├── SearchPanel.tsx         # Search overlay
│   │   └── DetailView/             # Detail views for block/tx/address
│   │       ├── BlockDetail.tsx
│   │       ├── TxDetail.tsx
│   │       ├── AddressDetail.tsx
│   │       └── HashArt.tsx         # Block hash generative art (128×128 canvas)
│   │
│   ├── hooks/                      # React hooks
│   │   ├── useChainStore.ts        # Zustand selector hooks
│   │   ├── useRPC.ts               # RPC query hooks
│   │   ├── useScene.ts             # Scene lifecycle management
│   │   └── useAmbientMode.ts       # Idle detection → ambient fade
│   │
│   └── utils/
│       ├── format.ts               # Address truncation, ETH formatting, hex
│       ├── color.ts                # Hash → ROSEDUST color derivation
│       └── math.ts                 # Lerp, clamp, golden ratio, perlin noise
```

---

## ROSEDUST Token Implementation

The complete CSS custom property set. This is the single source of truth.

```css
/* apps/explorer/src/design/tokens.css */
:root {
  /* ── Void (backgrounds) ── */
  --rd-void:            #060608;
  --rd-void-light:      #0c0c10;
  --rd-void-surface:    #12111a;

  /* ── Rose (primary accent — blocks, heartbeat) ── */
  --rd-rose-deep:       #3a2030;
  --rd-rose-dim:        #6a4058;
  --rd-rose:            #aa7088;
  --rd-rose-bright:     #cc90a8;
  --rd-rose-glow:       #dca5bd;

  /* ── Bone (value, transactions, warmth) ── */
  --rd-bone-dim:        #8a7860;
  --rd-bone:            #b8a880;
  --rd-bone-bright:     #d8c8a0;

  /* ── Dream (pending, unresolved, liminal) ── */
  --rd-dream-dim:       #5a5a78;
  --rd-dream:           #7a7a98;
  --rd-dream-bright:    #9494b4;

  /* ── Semantic ── */
  --rd-success:         #7a8a78;
  --rd-warning:         #c89a68;
  --rd-danger:          #cc5555;

  /* ── Text ── */
  --rd-text:            #c8b8c0;
  --rd-text-dim:        #6a5a68;
  --rd-text-ghost:      #3a303a;
  --rd-text-bright:     #e8d8e0;

  /* ── Borders ── */
  --rd-border:          rgba(255, 255, 255, 0.07);
  --rd-border-strong:   rgba(255, 255, 255, 0.14);
  --rd-border-rose:     rgba(170, 112, 136, 0.3);

  /* ── Glass surfaces ── */
  --rd-glass-bg:        rgba(8, 8, 12, 0.45);
  --rd-glass-bg-hover:  rgba(170, 112, 136, 0.08);
  --rd-glass-highlight: rgba(255, 255, 255, 0.06);
  --rd-glass-blur:      12px;
  --rd-glass-saturate:  180%;

  /* ── LED indicator ── */
  --rd-led-size:        5px;
  --rd-led-connected:   var(--rd-success);
  --rd-led-warning:     var(--rd-warning);
  --rd-led-error:       var(--rd-danger);
  --rd-led-streaming:   var(--rd-rose-glow);

  /* ── Spacing (8px base grid) ── */
  --rd-space-xs:        4px;
  --rd-space-sm:        8px;
  --rd-space-md:        16px;
  --rd-space-lg:        24px;
  --rd-space-xl:        32px;
  --rd-space-2xl:       48px;

  /* ── Typography ── */
  --rd-font-mono:       'JetBrains Mono', 'SF Mono', monospace;
  --rd-font-display:    'Fraunces', Georgia, serif;

  --rd-text-xs:         10px;
  --rd-text-sm:         11px;
  --rd-text-base:       14px;
  --rd-text-lg:         18px;
  --rd-text-xl:         24px;
  --rd-text-hero:       38px;

  --rd-tracking-tight:  0.02em;
  --rd-tracking-normal: 0.06em;
  --rd-tracking-wide:   0.12em;
  --rd-tracking-label:  0.28em;
  --rd-tracking-section: 0.32em;

  /* ── Transitions ── */
  --rd-ease-out:        cubic-bezier(0.16, 1, 0.3, 1);   /* expo ease-out */
  --rd-ease-in-out:     cubic-bezier(0.65, 0, 0.35, 1);
  --rd-duration-fast:   80ms;
  --rd-duration-normal: 200ms;
  --rd-duration-slow:   400ms;

  /* ── Z-index layers ── */
  --rd-z-scene:         1;
  --rd-z-panels:        100;
  --rd-z-overlay:       500;
  --rd-z-search:        600;
  --rd-z-detail:        700;
  --rd-z-grain:         9997;
  --rd-z-vignette:      9998;
  --rd-z-scanlines:     9999;

  /* ── Scene-specific ── */
  --rd-activity:        0;          /* 0-0.15, driven by JS */
  --rd-ambient-opacity: 1;         /* fades to 0.2 in ambient mode */
}
```

### Font Loading Strategy

```css
/* apps/explorer/src/design/typography.css */
@font-face {
  font-family: 'Fraunces';
  src: url('/fonts/Fraunces-Italic-Variable.woff2') format('woff2');
  font-style: italic;
  font-display: swap;
  font-weight: 100 900;
}

@font-face {
  font-family: 'JetBrains Mono';
  src: url('/fonts/JetBrainsMono-Variable.woff2') format('woff2');
  font-style: normal;
  font-display: swap;
  font-weight: 100 800;
}
```

Both fonts loaded as variable fonts for minimal network requests. `font-display: swap` ensures content renders immediately with fallback while fonts load. Total font weight: ~100KB (both woff2 combined).

### Glass Panel Component (CSS Module)

```css
/* apps/explorer/src/design/glass.module.css */
.panel {
  background: var(--rd-glass-bg);
  backdrop-filter: blur(var(--rd-glass-blur)) saturate(var(--rd-glass-saturate));
  -webkit-backdrop-filter: blur(var(--rd-glass-blur)) saturate(var(--rd-glass-saturate));
  border: 1px solid var(--rd-border);
  box-shadow: inset 0 1px 0 var(--rd-glass-highlight);
  border-radius: 0;
  position: relative;
  transition:
    transform var(--rd-duration-fast) var(--rd-ease-out),
    background var(--rd-duration-normal) var(--rd-ease-out);
}

.panel:hover {
  transform: translateY(-2px);
  background: var(--rd-glass-bg-hover);
  transition-duration: var(--rd-duration-fast), var(--rd-duration-fast);
}

/* Asymmetric hover-out: slower return */
.panel:not(:hover) {
  transition-duration: 120ms, var(--rd-duration-normal);
}

.panelActive {
  border-left: 2px solid var(--rd-rose-dim);
}

.label {
  font-family: var(--rd-font-mono);
  font-size: var(--rd-text-xs);
  text-transform: uppercase;
  letter-spacing: var(--rd-tracking-label);
  color: var(--rd-text-dim);
}

.led {
  width: var(--rd-led-size);
  height: var(--rd-led-size);
  border-radius: 50%;
  background: var(--rd-led-connected);
  box-shadow: 0 0 6px var(--rd-led-connected);
  animation: pulse 2.4s ease-in-out infinite;
}

@keyframes pulse {
  0%, 100% { opacity: 0.7; }
  50% { opacity: 1; }
}
```

### Atmosphere Implementation

```css
/* apps/explorer/src/design/atmosphere.css */

/* ── Grain ── */
.grain {
  position: fixed;
  inset: 0;
  pointer-events: none;
  z-index: var(--rd-z-grain);
  opacity: 0.035;
  mix-blend-mode: overlay;
  background-image: url("data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' width='300' height='300'%3E%3Cfilter id='g'%3E%3CfeTurbulence type='fractalNoise' baseFrequency='0.85' numOctaves='3' stitchTiles='stitch'/%3E%3C/filter%3E%3Crect width='100%25' height='100%25' filter='url(%23g)'/%3E%3C/svg%3E");
}

/* ── Vignette ── */
body::before {
  content: '';
  position: fixed;
  inset: 0;
  pointer-events: none;
  z-index: var(--rd-z-vignette);
  background: radial-gradient(ellipse at 50% 30%, transparent 50%, rgba(6, 6, 8, 0.72) 100%);
}

/* ── Scanlines ── */
body::after {
  content: '';
  position: fixed;
  inset: 0;
  pointer-events: none;
  z-index: var(--rd-z-scanlines);
  background: repeating-linear-gradient(
    to bottom,
    transparent 0,
    transparent 2px,
    rgba(0, 0, 0, 0.45) 2px,
    rgba(0, 0, 0, 0.45) 3px
  );
  opacity: 0.06;
  mix-blend-mode: multiply;
}

/* ── Rose Wash ── */
.roseWash {
  position: fixed;
  inset: 0;
  pointer-events: none;
  z-index: calc(var(--rd-z-panels) - 1);
  background: radial-gradient(
    ellipse at 50% 50%,
    rgba(170, 112, 136, var(--rd-activity)) 0%,
    transparent 60%
  );
  transition: opacity 2s ease-out;
}
```

Note: `--rd-activity` is updated by JavaScript via `document.documentElement.style.setProperty('--rd-activity', value)` when blocks arrive. Range 0 (idle) → 0.15 (active block with many transactions).

---

## Performance Budgets

| Metric | Target | Rationale |
|--------|--------|-----------|
| Initial load (LCP) | < 2.5s | Hero scene visible. Fonts may still be loading |
| JS bundle (gzip) | < 200KB (core) + 150KB (Three.js) | Code-split scenes as lazy chunks |
| 60 FPS (desktop) | Terrain + panels | All scenes must maintain 60fps on mid-range GPU |
| 30 FPS (mobile) | Mosaic only | No 3D on mobile, 2D tiles must be smooth |
| Memory | < 150MB | 50MB chain cache + 100MB GPU textures/buffers |
| WebSocket msgs | Handle 100/sec burst | Pending tx subscription can spike on busy chains |

### Bundle Strategy

```
main.js            (~80KB gz)   — React, Zustand, router, panels, data layer
terrain.js         (~120KB gz)  — Three.js + R3F + terrain shaders (lazy)
constellation.js   (~60KB gz)   — regl + particle system (lazy)
waterfall.js       (~20KB gz)   — Canvas2D waterfall scene (lazy)
detail.js          (~40KB gz)   — Detail views, hash art generator (lazy)
audio.js           (~30KB gz)   — Tone.js + chain sonification (lazy, off by default)
```

Scenes are code-split. Only the active scene's chunk is loaded. Mode 1 (TERRAIN) loads `terrain.js` on first render. Switching to Mode 2 (MOSAIC) doesn't need any scene chunk — it's Canvas2D inline.
