# Interaction Model

The explorer has two interaction postures: **ambient** (lean back, watch) and **investigative** (lean in, explore). The default is ambient. Interaction pulls you into investigative. Inactivity fades you back to ambient.

---

## Ambient Mode (Default)

The explorer runs unattended. No cursor visible, no panels highlighted. Just the scene,
the atmosphere, and the data flowing.

**What happens:**
- Terrain grows rightward, camera tracks slowly
- Constellation drifts, nodes pulse
- Waterfall blocks fall and stack
- Fee waveform scrolls
- Block pulse ticks

**No interaction required.** This is the "put it on the big screen in the office" mode.
It is visually interesting without anyone touching it.

**Idle timeout:** After 30 seconds of no mouse/keyboard activity, panels fade to 50% opacity
and eventually to 20%. The scene takes full visual priority. Atmospheric layers remain.

---

## Investigative Mode (On Interaction)

Any mouse movement or keypress transitions to investigative mode (200ms fade-in of panels).

### Mouse

| Action | Terrain Mode | Constellation Mode | Mosaic Mode |
|--------|-------------|-------------------|-------------|
| **Move** | Camera tilt (±5° lerp) | Highlight nearest node | Highlight nearest tile |
| **Click** | Nothing (terrain is ambient) | Select address → detail panel | Select block → detail panel |
| **Scroll** | Zoom in/out (camera Z) | Zoom in/out | Scroll grid |
| **Drag** | Pan terrain (camera X/Y) | Pan constellation | — |
| **Double-click** | — | Zoom to address | Expand tile to full detail |

### Hover States

All hover states follow ROSEDUST rules:
- `translateY(-2px)` on panels/tiles (small, not dramatic)
- `background: var(--bg-glass-hover)` (rose-tinted glass)
- Transition: 80ms on hover-in, 120ms on hover-out (asymmetric)
- Never `transform: scale()` on hover — only Y translate

**Address node hover (constellation):**
```
       ┌─ 0xa3f2...8b01 ──────┐
       │  12.45 ETH            │    ← appears 200ms after hover
       │  142 txns             │    ← JetBrains Mono 10px
       │  ● ACTIVE             │    ← LED + status
       └───────────────────────┘
             │
             ◉  ← node brightens, halo expands
```
Tooltip: glass panel, appears with `fadeUp` animation (opacity 0→1, translateY 4px→0, 200ms expo ease).
Offset: always above node, centered horizontally, clamped to viewport.

**Block tile hover (mosaic):**
Tile border brightens to `--border-strong`. Block number label appears inside tile.
Adjacent tiles dim slightly (0.7 opacity) to focus attention.

**Waterfall block hover:**
Block expands to show transaction detail lines. Mono labels appear.

### Keyboard

| Key | Action |
|-----|--------|
| `1` | Switch to TERRAIN mode |
| `2` | Switch to MOSAIC mode |
| `3` | Switch to DETAIL mode (last viewed entity) |
| `Space` | Pause/resume auto-scroll (terrain camera, waterfall) |
| `←` `→` | Step through blocks (when paused) |
| `/` | Focus search input |
| `Esc` | Close detail panel / exit search / return to ambient |
| `F` | Toggle fullscreen |

### Search

Triggered by `/` key or clicking the search area.

```
┌─ ⌕ ─────────────────────────────────────────────┐
│  0xa3f2...                                       │    ← JetBrains Mono 14px
│                                                  │    ← glass panel, auto-focus
│  RESULTS                                         │
│  ┌─ ADDRESS ──────────────────────────────────┐  │
│  │  0xa3f2...8b01    12.45 ETH    142 txns    │  │
│  └────────────────────────────────────────────┘  │
│  ┌─ BLOCK ────────────────────────────────────┐  │
│  │  #286,401    0 txns    1s ago              │  │
│  └────────────────────────────────────────────┘  │
└──────────────────────────────────────────────────┘
```

Search input accepts:
- Block number (decimal or hex)
- Transaction hash (0x...)
- Address (0x...)

Results appear instantly as you type (local cache first, then RPC fallback).
Select a result → navigate to detail view for that entity.

---

## Detail Views

### Block Detail

```
┌──────────────────────────────────────────────────────────────┐
│  ← BLOCKS                                   BLOCK 286,401   │
│                                                              │
│  ┌─ OVERVIEW ──────────────┐  ┌─ HASH ART ────────────────┐ │
│  │                         │  │                            │ │
│  │  HASH                   │  │  ┌────────────────────┐    │ │
│  │  0x6d1fb175dce4b679...  │  │  │                    │    │ │
│  │                         │  │  │  128×128 canvas    │    │ │
│  │  PARENT                 │  │  │  generative art    │    │ │
│  │  0x469162b1b3e2cc0b...  │  │  │  from block hash   │    │ │
│  │                         │  │  │                    │    │ │
│  │  STATE ROOT             │  │  │  (click to expand  │    │ │
│  │  0x3eef323af7ecaed3...  │  │  │   to fullscreen)   │    │ │
│  │                         │  │  │                    │    │ │
│  │  ┌─ METRICS ──────────┐ │  │  └────────────────────┘    │ │
│  │  │ GAS    │ FEE       │ │  │                            │ │
│  │  │ 0      │ 1 gwei    │ │  │  block.hash → visual      │ │
│  │  │ / 250M │           │ │  │  unique, deterministic     │ │
│  │  └────────┴───────────┘ │  │  downloadable as PNG       │ │
│  │                         │  │                            │ │
│  └─────────────────────────┘  └────────────────────────────┘ │
│                                                              │
│  —— TRANSACTIONS ──────────────────────────────────────────  │
│                                                              │
│  (empty — no transactions in this block)                     │
│                                                              │
│  ← BLOCK 286,400              BLOCK 286,402 →               │
│                                                              │
└──────────────────────────────────────────────────────────────┘
```

Hash values: JetBrains Mono 11px, text-dim, click to copy (flash bone on copy).
Metric cells: standard ROSEDUST mosaic component (1px gap grid, glass bg).
Navigation: arrow keys or click to step through blocks.

### Address Detail

```
┌──────────────────────────────────────────────────────────────┐
│  ← ADDRESSES                  0xa3f2...8b01                 │
│                                                              │
│  ┌─ IDENTITY ──────────────┐  ┌─ CONSTELLATION ────────────┐│
│  │                         │  │                            ││
│  │  BALANCE                │  │  zoomed-in view of this    ││
│  │  12.450000 ETH          │  │  address in the            ││
│  │                         │  │  constellation, showing    ││
│  │  NONCE                  │  │  connected addresses       ││
│  │  142                    │  │  and recent arcs           ││
│  │                         │  │                            ││
│  │  TYPE                   │  │         ◌                  ││
│  │  ◈ CONTRACT             │  │       ╱   ╲               ││
│  │                         │  │     ◉ ← you  ◌            ││
│  │  CODE SIZE              │  │       ╲   ╱               ││
│  │  4,892 bytes            │  │         ◌                  ││
│  │                         │  │                            ││
│  └─────────────────────────┘  └────────────────────────────┘│
│                                                              │
│  —— RECENT TRANSACTIONS ───────────────────────────────────  │
│                                                              │
│  HASH              TO              VALUE        GAS          │
│  0x1a2b...  →  0xc3d4...8901   0.5 ETH     21,000          │
│  0x5e6f...  →  0x7a8b...2345   1.0 ETH     21,000          │
│  ...                                                         │
│                                                              │
└──────────────────────────────────────────────────────────────┘
```

Balance: Fraunces italic 38px, bone-bright (the hero number).
Transaction table: standard ROSEDUST table component (hover rows, stagger entrance).

### Transaction Detail

```
┌──────────────────────────────────────────────────────────────┐
│  ← BLOCK 286,401                              TX 0x1a2b...  │
│                                                              │
│  ┌─ FLOW ──────────────────────────────────────────────────┐ │
│  │                                                          │ │
│  │    0xa3f2...8b01  ──────── 0.5 ETH ────────▶  0xc3d4... │ │
│  │    SENDER                                     RECEIVER   │ │
│  │                                                          │ │
│  └──────────────────────────────────────────────────────────┘ │
│                                                              │
│  ┌─ DATA ──────────────┐  ┌─ EXECUTION ───────────────────┐ │
│  │                     │  │                                │ │
│  │  STATUS  ● SUCCESS  │  │  GAS LIMIT     21,000         │ │
│  │  BLOCK   286,401    │  │  GAS USED      21,000         │ │
│  │  INDEX   0          │  │  GAS PRICE     1 gwei         │ │
│  │  NONCE   141        │  │  FEE PAID      0.000021 ETH   │ │
│  │  TYPE    0x2 (1559) │  │                                │ │
│  │  VALUE   0.5 ETH    │  │  ┌──────────────────────┐     │ │
│  │                     │  │  │ ██████████████████ █ │     │ │
│  │                     │  │  │ gas usage bar       │     │ │
│  │                     │  │  └──────────────────────┘     │ │
│  │                     │  │                                │ │
│  └─────────────────────┘  └────────────────────────────────┘ │
│                                                              │
│  —— CALL TRACE (requires debug_traceTransaction) ──────────  │
│                                                              │
│  CALL  0xa3f2... → 0xc3d4...  value: 0.5 ETH               │
│    └─ SSTORE  slot 0x01 = 0x...                             │
│    └─ LOG     Transfer(from, to, amount)                    │
│                                                              │
└──────────────────────────────────────────────────────────────┘
```

Flow diagram: bone-bright arc with animated particles (reuses constellation arc renderer).
Call trace (if available): tree structure, ROSEDUST table style, each opcode type color-coded.

---

## Transitions

### Mode Switch (1 → 2 → 3)

1. Current scene fades out (opacity 1→0, 300ms)
2. 100ms gap (void visible — intentional beat)
3. New scene fades in (opacity 0→1, translateY 12px→0, 300ms expo ease)

Total: 700ms. Feels deliberate, not instant. Not sluggish.

### Detail Panel Open

1. Clicked element pulses once (rose-glow flash, 200ms)
2. Background scene dims to 30% opacity (300ms)
3. Detail panel slides in from right (translateX 100%→0, 400ms expo ease)

### Detail Panel Close

1. Panel slides out (translateX 0→100%, 300ms)
2. Background scene restores to 100% (300ms, overlapping)
3. If came from a specific element, camera smoothly returns to its position

### Block Arrival (Ambient)

Every 1 second:
1. Terrain: new tile fades in at right edge (opacity 0→1, 400ms)
2. Waterfall: new block falls from top (400ms expo deceleration)
3. Pulse: current dot flashes (80ms)
4. Status panel: block number increments (value-flash animation, 300ms)
5. If block has transactions: rose-wash intensity pulses (0→0.15→0, 2s)

All 5 happen simultaneously. The block arrival is a *chord*, not a sequence.

---

## Touch (Mobile/Tablet)

| Gesture | Action |
|---------|--------|
| Tap | Select (same as click) |
| Long press | Show tooltip (same as hover) |
| Pinch | Zoom |
| Two-finger drag | Pan |
| Swipe left/right | Navigate between blocks (detail view) |
| Swipe down | Close detail panel |

No 3D scenes on mobile — defaults to MOSAIC mode with hash-art tiles.
Tiles are tap targets (minimum 44×44px). Detail panels are full-screen on mobile.

---

## Audio (Optional, Off by Default)

Toggle with speaker icon in status bar.

| Event | Sound |
|-------|-------|
| Block arrival | Soft click / tick (wood block, 50ms) |
| Transaction confirmed | Gentle chime, pitch ∝ value (higher value = higher pitch) |
| Transaction pending | Low hum note (sustained while pending) |
| Transaction reverted | Dull thud |
| Connection lost | Ambient drone fades out |
| Connection restored | Ambient drone fades in |

Ambient layer: continuous generative drone.
Base frequency: 55Hz (A1). Harmonics added based on chain activity.
Gas pressure modulates filter cutoff. More gas = brighter timbre.
Empty chain = deep, dark, minimal. Busy chain = rich, shimmering.

Implementation: Web Audio API, OscillatorNode + BiquadFilterNode.
No audio files — everything synthesized from chain state.

---

## URL Routing

Every navigable state in the explorer maps to a URL for shareability and deep linking.

```typescript
// apps/explorer/src/router.tsx
import { createBrowserRouter } from 'react-router-dom';

export const router = createBrowserRouter([
  {
    path: '/',
    element: <App />,
    children: [
      // Mode 1: Terrain (default)
      { index: true, element: <TerrainMode /> },

      // Mode 2: Mosaic
      { path: 'mosaic', element: <MosaicMode /> },

      // Mode 3: Detail views
      { path: 'block/:numberOrHash', element: <BlockDetail /> },
      { path: 'tx/:hash', element: <TxDetail /> },
      { path: 'address/:address', element: <AddressDetail /> },

      // Optional: Consensus view
      { path: 'consensus', element: <ConsensusView /> },
    ],
  },
]);

// URL examples:
// /                          → Terrain mode (default)
// /mosaic                    → Mosaic mode
// /block/286401              → Block detail by number
// /block/0x6d1f...           → Block detail by hash
// /tx/0x1a2b...              → Transaction detail
// /address/0xa3f2...         → Address detail
// /consensus                 → Consensus ring view
```

### Navigation State Machine

```
                    ┌─────────────────┐
          ┌────────│   TERRAIN (/)    │────────┐
          │        └────────┬────────┘        │
          │   key "2"       │  click entity    │  key "1"
          │                 │                  │
          ▼                 ▼                  ▼
  ┌───────────────┐  ┌─────────────────┐  ┌───────────────┐
  │ MOSAIC        │  │ DETAIL          │  │ TERRAIN       │
  │ /mosaic       │◄─│ /block/:id      │──│ (return)      │
  │               │  │ /tx/:hash       │  │               │
  └───────┬───────┘  │ /address/:addr  │  └───────────────┘
          │          └────────┬────────┘
          │   click tile      │  Esc / ← BACK
          │                   │
          └───────────────────┘
```

Transitions between modes always use `router.navigate()` so the URL stays in sync. The browser back button naturally walks backward through the navigation history.

---

## Ambient / Investigative State Machine

```typescript
// apps/explorer/src/hooks/useAmbientMode.ts
import { useEffect, useRef } from 'react';
import { useUIStore } from '../data/store';

const IDLE_TIMEOUT = 30_000;     // 30s to enter ambient
const FADE_DURATION = 2_000;     // 2s opacity fade
const PANEL_DIM_OPACITY = 0.2;   // panels nearly invisible in ambient

export function useAmbientMode() {
  const setAmbient = useUIStore((s) => s.setAmbient);
  const timerRef = useRef<number>();
  const fadeRef = useRef<number>();

  useEffect(() => {
    const resetTimer = () => {
      // Any interaction → investigative mode
      setAmbient(false);
      clearTimeout(timerRef.current);
      cancelAnimationFrame(fadeRef.current!);

      timerRef.current = window.setTimeout(() => {
        // No interaction for 30s → begin ambient fade
        fadeToAmbient(setAmbient);
      }, IDLE_TIMEOUT);
    };

    // Track all interaction events
    const events = ['mousemove', 'mousedown', 'keydown', 'touchstart', 'scroll'];
    events.forEach((e) => window.addEventListener(e, resetTimer, { passive: true }));
    resetTimer(); // initial setup

    return () => {
      events.forEach((e) => window.removeEventListener(e, resetTimer));
      clearTimeout(timerRef.current);
    };
  }, [setAmbient]);
}

function fadeToAmbient(setAmbient: (active: boolean) => void) {
  setAmbient(true);
  // CSS transition handles the actual opacity animation:
  // .panels { opacity: var(--rd-ambient-opacity); transition: opacity 2s ease-out; }
}
```

### State Transitions

| State | Panels Opacity | Scene Brightness | Cursor | Triggered By |
|-------|---------------|-----------------|--------|-------------|
| Investigative | 100% | 100% | Visible | Any input event |
| Ambient (transitioning) | 100% → 20% | 100% | Fading | 30s idle timer fires |
| Ambient (steady) | 20% | 100% | Hidden | Timer completed |
| Back to Investigative | 20% → 100% (200ms) | 100% | Visible | Mouse/key/touch |

---

## Accessibility Considerations

### Keyboard Navigation

All interactive elements are reachable via Tab. Focus ring: 2px `--rd-rose` outline with 2px offset (visible against dark backgrounds). Focus follows the ROSEDUST aesthetic — no browser-default blue outline.

```css
:focus-visible {
  outline: 2px solid var(--rd-rose);
  outline-offset: 2px;
}
```

### Screen Reader Support

- All glass panels have `role="region"` with `aria-label` describing their content
- Block numbers and transaction hashes are in `<code>` tags with `aria-label` for full values
- Mode switches are `role="tablist"` with `aria-selected`
- Scene canvases have `aria-hidden="true"` (purely visual, data available in panels)
- Status LED has `aria-live="polite"` for connection state changes
- Search results have `role="listbox"` with `aria-activedescendant`

### Reduced Motion

```css
@media (prefers-reduced-motion: reduce) {
  /* Disable atmospheric animations */
  .grain, body::before, body::after { display: none; }

  /* Disable scene animations — show static snapshots */
  canvas { animation: none !important; }

  /* Instant transitions instead of animated */
  * {
    transition-duration: 0ms !important;
    animation-duration: 0ms !important;
  }

  /* Waterfall blocks appear instantly, no falling animation */
  /* Constellation particles don't animate, show connections as static lines */
  /* Terrain doesn't scroll — shows latest 30 tiles as static landscape */
}
```

### Color Contrast

The ROSEDUST palette is designed for dark backgrounds. Key contrast ratios:
- `--rd-text` (#c8b8c0) on `--rd-void` (#060608): **9.4:1** (AAA)
- `--rd-text-dim` (#6a5a68) on `--rd-void`: **3.2:1** (AA Large)
- `--rd-bone-bright` (#d8c8a0) on `--rd-glass-bg`: **8.1:1** (AAA)
- `--rd-rose` (#aa7088) on `--rd-void`: **4.8:1** (AA)

`--rd-text-ghost` (#3a303a) intentionally fails contrast requirements — it's decorative, not informational. Any text using `text-ghost` must have a visible alternative nearby.

---

## Search Implementation

```typescript
// apps/explorer/src/panels/SearchPanel.tsx

// Search is local-first, RPC-fallback:
// 1. Type in query
// 2. Debounce 150ms
// 3. Check local cache first:
//    - Block number? → cache.blocks.get(BigInt(query))
//    - Hex string 66 chars (0x + 64)? → cache.blocks (by hash) or pending txs
//    - Hex string 42 chars (0x + 40)? → cache.addresses.get(query)
// 4. If no local match and query looks valid:
//    - Block number → rpc.getBlockByNumber()
//    - Tx hash → rpc.getTransactionByHash()
//    - Address → rpc.getBalance() + rpc.getCode() (to determine type)
// 5. Results categorized: BLOCK | TRANSACTION | ADDRESS
// 6. Arrow keys navigate results, Enter selects → navigates to detail view

interface SearchResult {
  type: 'block' | 'transaction' | 'address';
  label: string;        // "Block 286,401" or "0xa3f2...8b01"
  subtitle: string;     // "0 txns · 1s ago" or "12.45 ETH"
  source: 'cache' | 'rpc';
  navigateTo: string;   // URL path for router
}

// Performance: local cache search is O(1) for exact matches.
// RPC fallback adds 100-300ms latency.
// Results appear as you type — cache matches are instant,
// RPC matches fade in when they resolve.
```

---

## Animation Performance Targets

| Animation | Target FPS | Max Frame Budget | Technique |
|-----------|-----------|-----------------|-----------|
| Terrain camera tracking | 60 | 16.6ms | requestAnimationFrame + Three.js |
| Particle constellation | 60 | 16.6ms | regl transform feedback (GPU) |
| Block waterfall fall | 60 | 16.6ms | CSS transform + will-change |
| Panel hover (translateY) | 60 | 16.6ms | CSS transition (compositor) |
| Mode crossfade | 60 | 16.6ms | CSS opacity (compositor) |
| Detail panel slide | 60 | 16.6ms | Framer Motion (transform) |
| Ambient opacity fade | 30+ | 33ms | CSS transition (slow, no precision needed) |
| Search result list | 60 | 16.6ms | React DOM (small list, <20 items) |

### Compositor-Only Properties

All hover/transition animations use only `transform` and `opacity` (compositor-only, no layout/paint):

```css
/* These trigger compositor — fast, off-main-thread */
.panel:hover { transform: translateY(-2px); }
.fadeIn { opacity: 0 → 1; transform: translateY(12px) → translateY(0); }

/* NEVER do this — triggers layout recalculation */
/* .panel:hover { margin-top: -2px; } ← BAD */
/* .fadeIn { height: 0 → auto; } ← BAD */
```

### GPU Memory Monitoring

```typescript
// Monitor WebGL context memory (where available)
function checkGPUMemory() {
  const gl = canvas.getContext('webgl2');
  const ext = gl?.getExtension('WEBGL_debug_renderer_info');
  if (ext) {
    const renderer = gl.getParameter(ext.UNMASKED_RENDERER_WEBGL);
    console.info('GPU:', renderer);
  }

  // If device has < 4GB RAM, reduce:
  // - Terrain ring buffer: 200 → 100 tiles
  // - Particle max: 16384 → 8192
  // - Block cache: 500 → 200
  if (navigator.deviceMemory && navigator.deviceMemory < 4) {
    applyLowMemoryProfile();
  }
}
```
