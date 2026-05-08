# 07 — Scene 3: Live Block Waterfall + Gas Waveform

Scene 3 renders blocks as horizontal bars that scroll vertically (newest at top), with a gas usage waveform below. Pure Canvas2D -- no WebGL, no Three.js. This is the fastest scene to build and the first to prove the full data pipeline end-to-end.

**Depends on:** [`02-rosedust-tokens.md`](./02-rosedust-tokens.md), [`04-state-stores.md`](./04-state-stores.md)

---

## Architecture Overview

```
┌──────────────────────────────────────────────────────┐
│                                                      │
│  WaterfallScene.tsx (React wrapper)                  │
│  ├── <canvas ref={waterfallRef} />  ← main canvas   │
│  │     Canvas2D, full width, 50vh                    │
│  │     requestAnimationFrame loop                    │
│  │     Renders: block bars, barcodes, labels         │
│  │                                                   │
│  ├── <canvas ref={waveformRef} />   ← OffscreenCanvas│
│  │     Gas waveform, full width, 120px               │
│  │     Bezier-interpolated line chart                │
│  │                                                   │
│  └── Tooltip overlay (DOM, absolute positioned)      │
│                                                      │
└──────────────────────────────────────────────────────┘
```

**Why Canvas2D, not WebGL:**
- Block bars are simple rects with fills -- no mesh geometry, no shaders needed
- Canvas2D text rendering is native -- no font atlas texture, no SDF
- Barcode patterns are trivial fills -- no shader compilation
- Total draw calls per frame: ~130 rects + ~130 text draws + 1 waveform path
- Target: <2ms per frame on any hardware that can run a browser

**Why OffscreenCanvas for the waveform:**
- The waveform redraws only when fee data updates (every 5s) or on new block
- OffscreenCanvas runs in a worker, freeing the main thread entirely
- Falls back to a regular canvas on browsers without OffscreenCanvas support

---

## 7.1 Waterfall Canvas Component

### File: `apps/explorer/src/scenes/waterfall/WaterfallScene.tsx`

```tsx
import { useRef, useEffect, useCallback } from 'react';
import { bus } from '../../bus/events';
import { useChainStore } from '../../stores/chain';
import type { ChainBlock } from '../../rpc/types';
import { drawBlockBar, BAR_HEIGHT, BAR_GAP } from './blockBar';
import { GasWaveform } from './gasWaveform';
import { WaterfallTooltip, type TooltipData } from './tooltip';

/** Configuration constants */
const RING_SIZE = 128;           // max blocks in memory
const SCROLL_LERP = 0.12;       // smooth scroll interpolation factor
const AUTO_SCROLL_THRESHOLD = 2; // bars from top before auto-scroll re-engages

export function WaterfallScene() {
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const waveformRef = useRef<HTMLCanvasElement>(null);
  const containerRef = useRef<HTMLDivElement>(null);

  // Mutable state refs (not React state -- avoid re-renders in animation loop)
  const blocksRef = useRef<ChainBlock[]>([]);
  const scrollOffsetRef = useRef(0);       // current smooth scroll Y
  const targetScrollRef = useRef(0);       // target scroll Y (jumps on new block)
  const autoScrollRef = useRef(true);      // true = lock to newest
  const userScrollingRef = useRef(false);
  const rafIdRef = useRef(0);
  const tooltipRef = useRef<TooltipData | null>(null);
  const tooltipSetRef = useRef<(d: TooltipData | null) => void>(() => {});

  // Device pixel ratio for retina
  const dprRef = useRef(Math.min(window.devicePixelRatio || 1, 2));

  /** Resize canvas to match container, accounting for DPR */
  const resizeCanvas = useCallback(() => {
    const canvas = canvasRef.current;
    const container = containerRef.current;
    if (!canvas || !container) return;

    const dpr = dprRef.current;
    const rect = container.getBoundingClientRect();

    canvas.width = rect.width * dpr;
    canvas.height = rect.height * dpr;
    canvas.style.width = `${rect.width}px`;
    canvas.style.height = `${rect.height}px`;

    const ctx = canvas.getContext('2d');
    if (ctx) ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
  }, []);

  /** Main render loop */
  const render = useCallback(() => {
    const canvas = canvasRef.current;
    if (!canvas) return;

    // Skip frame if tab not visible
    if (document.hidden) {
      rafIdRef.current = requestAnimationFrame(render);
      return;
    }

    const ctx = canvas.getContext('2d');
    if (!ctx) return;

    const dpr = dprRef.current;
    const width = canvas.width / dpr;
    const height = canvas.height / dpr;
    const blocks = blocksRef.current;

    // Smooth scroll interpolation
    const currentScroll = scrollOffsetRef.current;
    const targetScroll = targetScrollRef.current;
    scrollOffsetRef.current = currentScroll + (targetScroll - currentScroll) * SCROLL_LERP;

    // Clear
    ctx.clearRect(0, 0, width, height);

    // Determine visible range
    const scrollY = scrollOffsetRef.current;
    const rowHeight = BAR_HEIGHT + BAR_GAP;
    const firstVisible = Math.max(0, Math.floor(scrollY / rowHeight));
    const lastVisible = Math.min(
      blocks.length - 1,
      Math.ceil((scrollY + height) / rowHeight),
    );

    // Draw only visible block bars
    for (let i = firstVisible; i <= lastVisible; i++) {
      const block = blocks[i];
      if (!block) continue;

      const y = i * rowHeight - scrollY;
      drawBlockBar(ctx, block, y, width);
    }

    // Draw scroll indicator (right edge)
    if (blocks.length > 0) {
      const totalHeight = blocks.length * rowHeight;
      const viewRatio = height / totalHeight;
      if (viewRatio < 1) {
        const thumbHeight = Math.max(20, height * viewRatio);
        const thumbY = (scrollY / totalHeight) * height;

        ctx.fillStyle = 'rgba(170, 112, 136, 0.25)'; // --rd-rose at 25%
        ctx.fillRect(width - 4, thumbY, 3, thumbHeight);
      }
    }

    rafIdRef.current = requestAnimationFrame(render);
  }, []);

  /** Handle new block from EventBus */
  const onNewBlock = useCallback((block: ChainBlock) => {
    const blocks = blocksRef.current;

    // Prepend (newest first)
    blocks.unshift(block);

    // Ring buffer: evict oldest if over capacity
    if (blocks.length > RING_SIZE) {
      blocks.pop();
    }

    // Auto-scroll: shift target so newest block is at top
    if (autoScrollRef.current) {
      targetScrollRef.current = 0;
    } else {
      // User is scrolled down -- shift target to keep their position stable
      targetScrollRef.current += BAR_HEIGHT + BAR_GAP;
    }
  }, []);

  /** Mouse wheel / trackpad scroll */
  const onWheel = useCallback((e: WheelEvent) => {
    e.preventDefault();
    userScrollingRef.current = true;

    const rowHeight = BAR_HEIGHT + BAR_GAP;
    const maxScroll = Math.max(0, blocksRef.current.length * rowHeight - (canvasRef.current?.height ?? 0) / dprRef.current);

    targetScrollRef.current = Math.max(0, Math.min(maxScroll, targetScrollRef.current + e.deltaY));

    // Re-engage auto-scroll if near top
    if (targetScrollRef.current < AUTO_SCROLL_THRESHOLD * rowHeight) {
      autoScrollRef.current = true;
      targetScrollRef.current = 0;
    } else {
      autoScrollRef.current = false;
    }

    // Clear user scrolling flag after a delay
    clearTimeout((onWheel as any)._timeout);
    (onWheel as any)._timeout = setTimeout(() => {
      userScrollingRef.current = false;
    }, 150);
  }, []);

  /** Hover detection: find which block bar the mouse is over */
  const onMouseMove = useCallback((e: MouseEvent) => {
    const canvas = canvasRef.current;
    if (!canvas) return;

    const rect = canvas.getBoundingClientRect();
    const x = e.clientX - rect.left;
    const y = e.clientY - rect.top;

    const scrollY = scrollOffsetRef.current;
    const rowHeight = BAR_HEIGHT + BAR_GAP;
    const index = Math.floor((y + scrollY) / rowHeight);
    const blocks = blocksRef.current;

    if (index >= 0 && index < blocks.length) {
      const block = blocks[index];
      tooltipSetRef.current({
        block,
        x: e.clientX,
        y: e.clientY,
        barY: index * rowHeight - scrollY,
      });
    } else {
      tooltipSetRef.current(null);
    }
  }, []);

  /** Click: emit entity:select for BlockDetail navigation */
  const onClick = useCallback((e: MouseEvent) => {
    const canvas = canvasRef.current;
    if (!canvas) return;

    const rect = canvas.getBoundingClientRect();
    const y = e.clientY - rect.top;

    const scrollY = scrollOffsetRef.current;
    const rowHeight = BAR_HEIGHT + BAR_GAP;
    const index = Math.floor((y + scrollY) / rowHeight);
    const blocks = blocksRef.current;

    if (index >= 0 && index < blocks.length) {
      bus.emit('entity:select', {
        type: 'block',
        number: blocks[index].number,
      });
    }
  }, []);

  // Lifecycle
  useEffect(() => {
    resizeCanvas();

    const canvas = canvasRef.current;
    if (canvas) {
      canvas.addEventListener('wheel', onWheel, { passive: false });
      canvas.addEventListener('mousemove', onMouseMove);
      canvas.addEventListener('click', onClick);
      canvas.addEventListener('mouseleave', () => tooltipSetRef.current(null));
    }

    // Backfill from chain store (existing blocks)
    const existingBlocks = useChainStore.getState().recentBlocks(RING_SIZE);
    blocksRef.current = [...existingBlocks];

    // Subscribe to new blocks
    bus.on('block:new', onNewBlock);

    // Resize observer
    const observer = new ResizeObserver(resizeCanvas);
    if (containerRef.current) observer.observe(containerRef.current);

    // Start render loop
    rafIdRef.current = requestAnimationFrame(render);

    return () => {
      cancelAnimationFrame(rafIdRef.current);
      bus.off('block:new', onNewBlock);
      observer.disconnect();

      if (canvas) {
        canvas.removeEventListener('wheel', onWheel);
        canvas.removeEventListener('mousemove', onMouseMove);
        canvas.removeEventListener('click', onClick);
      }
    };
  }, [resizeCanvas, render, onNewBlock, onWheel, onMouseMove, onClick]);

  return (
    <div
      ref={containerRef}
      style={{
        width: '100%',
        height: '50vh',
        position: 'relative',
        overflow: 'hidden',
      }}
    >
      <canvas
        ref={canvasRef}
        style={{
          display: 'block',
          width: '100%',
          height: '100%',
          cursor: 'pointer',
        }}
      />
      <GasWaveform ref={waveformRef} />
      <WaterfallTooltip setRef={tooltipSetRef} />
    </div>
  );
}
```

### Checklist

- [ ] `WaterfallScene.tsx` compiles and mounts in React tree
- [ ] `useRef` for canvas element, `useEffect` for lifecycle
- [ ] `requestAnimationFrame` loop running at monitor refresh rate
- [ ] Canvas fills full container width, 50vh height
- [ ] DPR scaling: canvas internal resolution = CSS size x devicePixelRatio (capped at 2x)
- [ ] ResizeObserver handles window resize and layout shifts
- [ ] Frame skip when `document.hidden === true`

---

## 7.2 Block Bar Rendering

### File: `apps/explorer/src/scenes/waterfall/blockBar.ts`

Each block renders as a horizontal bar. Width encodes gas utilization. Color encodes transaction density. Labels show block number and tx count.

```typescript
import type { ChainBlock } from '../../rpc/types';
import { hashToBarcode } from './hashbar';

/** Layout constants */
export const BAR_HEIGHT = 24;  // px, logical (before DPR)
export const BAR_GAP = 2;     // px between bars
const LABEL_PADDING = 8;      // px from bar edge to label text
const NUMBER_WIDTH = 72;      // px reserved for block number label

/** ROSEDUST color palette for gas interpolation */
const COLOR_EMPTY: [number, number, number] = [0x12, 0x11, 0x1a]; // --rd-void-surface
const COLOR_LOW:   [number, number, number] = [0x3a, 0x20, 0x30]; // --rd-rose-deep
const COLOR_MID:   [number, number, number] = [0xaa, 0x70, 0x88]; // --rd-rose
const COLOR_FULL:  [number, number, number] = [0xdc, 0xa5, 0xbd]; // --rd-rose-glow

/**
 * Interpolate between ROSEDUST colors based on gas ratio.
 *
 * 0.00       -> void-surface (empty block, barely visible)
 * 0.00-0.33  -> void-surface to rose-deep
 * 0.33-0.66  -> rose-deep to rose
 * 0.66-1.00  -> rose to rose-glow
 */
export function gasToColor(gasRatio: number): string {
  let r: number, g: number, b: number;

  if (gasRatio < 0.33) {
    const t = gasRatio / 0.33;
    r = lerp(COLOR_EMPTY[0], COLOR_LOW[0], t);
    g = lerp(COLOR_EMPTY[1], COLOR_LOW[1], t);
    b = lerp(COLOR_EMPTY[2], COLOR_LOW[2], t);
  } else if (gasRatio < 0.66) {
    const t = (gasRatio - 0.33) / 0.33;
    r = lerp(COLOR_LOW[0], COLOR_MID[0], t);
    g = lerp(COLOR_LOW[1], COLOR_MID[1], t);
    b = lerp(COLOR_LOW[2], COLOR_MID[2], t);
  } else {
    const t = (gasRatio - 0.66) / 0.34;
    r = lerp(COLOR_MID[0], COLOR_FULL[0], t);
    g = lerp(COLOR_MID[1], COLOR_FULL[1], t);
    b = lerp(COLOR_MID[2], COLOR_FULL[2], t);
  }

  return `rgb(${Math.round(r)}, ${Math.round(g)}, ${Math.round(b)})`;
}

function lerp(a: number, b: number, t: number): number {
  return a + (b - a) * Math.max(0, Math.min(1, t));
}

/**
 * Draw a single block bar on the waterfall canvas.
 *
 * Layout:
 * ┌────────┬──────────────────────────────────────────┬────────┐
 * │ #286401│▓▓▓▓▓▓▓▓▓▓▓▓▓ barcode fill ▓▓▓▓▓▓▓▓▓▓▓▓│  3 txn │
 * └────────┴──────────────────────────────────────────┴────────┘
 *  ← number  ← gas-proportional width (barcode inside) → tx count →
 */
export function drawBlockBar(
  ctx: CanvasRenderingContext2D,
  block: ChainBlock,
  y: number,
  canvasWidth: number,
): void {
  const gasLimit = Number(block.gasLimit);
  const gasUsed = Number(block.gasUsed);
  const gasRatio = gasLimit > 0 ? gasUsed / gasLimit : 0;
  const txCount = block.transactions.length;

  // Bar dimensions
  const barAreaWidth = canvasWidth - NUMBER_WIDTH - 56; // 56px right label area
  const barWidth = Math.max(4, gasRatio * barAreaWidth); // minimum 4px even for empty
  const barX = NUMBER_WIDTH;

  // 1. Background: full-width subtle row stripe (alternating)
  const blockNum = Number(block.number);
  if (blockNum % 2 === 0) {
    ctx.fillStyle = 'rgba(255, 255, 255, 0.015)';
    ctx.fillRect(0, y, canvasWidth, BAR_HEIGHT);
  }

  // 2. Gas bar fill
  ctx.fillStyle = gasToColor(gasRatio);
  ctx.fillRect(barX, y + 1, barWidth, BAR_HEIGHT - 2);

  // 3. Hash barcode overlay (inside the bar)
  if (barWidth > 20) {
    hashToBarcode(block.hash, barWidth, BAR_HEIGHT - 2, ctx, barX, y + 1);
  }

  // 4. Border: 1px bottom edge
  ctx.fillStyle = 'rgba(255, 255, 255, 0.04)';
  ctx.fillRect(barX, y + BAR_HEIGHT - 1, barWidth, 1);

  // 5. Hover highlight bar (2px left accent if block has transactions)
  if (txCount > 0) {
    ctx.fillStyle = '#dca5bd'; // --rd-rose-glow
    ctx.fillRect(barX, y + 1, 2, BAR_HEIGHT - 2);
  }

  // 6. Left label: block number (monospace)
  ctx.font = '10px "JetBrains Mono", monospace';
  ctx.textAlign = 'right';
  ctx.textBaseline = 'middle';
  ctx.fillStyle = '#6a5a68'; // --rd-text-dim
  ctx.fillText(
    blockNum.toLocaleString(),
    NUMBER_WIDTH - LABEL_PADDING,
    y + BAR_HEIGHT / 2,
  );

  // 7. Right label: tx count (only if > 0)
  if (txCount > 0) {
    ctx.textAlign = 'left';
    ctx.fillStyle = '#d8c8a0'; // --rd-bone-bright
    ctx.fillText(
      `${txCount} tx`,
      barX + barWidth + LABEL_PADDING,
      y + BAR_HEIGHT / 2,
    );
  }

  // 8. Gas percentage (inside bar, right-aligned, if bar is wide enough)
  if (barWidth > 80) {
    ctx.textAlign = 'right';
    ctx.fillStyle = 'rgba(255, 255, 255, 0.4)';
    ctx.fillText(
      `${(gasRatio * 100).toFixed(1)}%`,
      barX + barWidth - 6,
      y + BAR_HEIGHT / 2,
    );
  }
}

/**
 * Draw a highlighted version of a block bar (on hover).
 * Same layout but brighter colors and a border.
 */
export function drawBlockBarHighlight(
  ctx: CanvasRenderingContext2D,
  block: ChainBlock,
  y: number,
  canvasWidth: number,
): void {
  const gasLimit = Number(block.gasLimit);
  const gasUsed = Number(block.gasUsed);
  const gasRatio = gasLimit > 0 ? gasUsed / gasLimit : 0;
  const barAreaWidth = canvasWidth - NUMBER_WIDTH - 56;
  const barWidth = Math.max(4, gasRatio * barAreaWidth);
  const barX = NUMBER_WIDTH;

  // Highlight background
  ctx.fillStyle = 'rgba(170, 112, 136, 0.08)';
  ctx.fillRect(0, y, canvasWidth, BAR_HEIGHT);

  // Highlight border
  ctx.strokeStyle = 'rgba(170, 112, 136, 0.3)';
  ctx.lineWidth = 1;
  ctx.strokeRect(barX - 0.5, y + 0.5, barWidth + 1, BAR_HEIGHT - 1);
}
```

### Checklist

- [ ] `drawBlockBar(ctx, block, y, width)` renders a complete block bar
- [ ] Bar position: `y = index * (BAR_HEIGHT + BAR_GAP) - scrollOffset`
- [ ] Bar width: `(block.gasUsed / block.gasLimit) * barAreaWidth`, minimum 4px
- [ ] Color interpolation: void-surface -> rose-deep -> rose -> rose-glow
- [ ] Block number label: left-aligned, monospace, `--rd-text-dim`
- [ ] Tx count label: right of bar, `--rd-bone-bright`, only shown when > 0
- [ ] Gas percentage: inside bar, right-aligned, only shown when bar > 80px wide
- [ ] 2px rose accent on left edge for blocks with transactions
- [ ] Alternating row stripe at 1.5% white opacity
- [ ] Highlight variant for hover state with border

---

## 7.3 Hash Barcode

### File: `apps/explorer/src/scenes/waterfall/hashbar.ts`

Each block's hash renders as a visual barcode inside its bar. The barcode acts as a visual fingerprint -- every block looks unique even at a glance.

```typescript
/**
 * Render a block hash as a barcode pattern inside a rectangular area.
 *
 * Algorithm:
 * - Split the 32-byte hash into individual nibbles (64 nibbles)
 * - Each nibble (0-15) determines a bar's relative width
 * - Bars alternate between two subtle color variations
 * - The result is a unique visual fingerprint for every hash
 *
 * @param hash - Block hash hex string (with or without 0x prefix)
 * @param width - Available width in pixels
 * @param height - Available height in pixels
 * @param ctx - Canvas2D rendering context
 * @param offsetX - X offset to start drawing
 * @param offsetY - Y offset to start drawing
 */
export function hashToBarcode(
  hash: string,
  width: number,
  height: number,
  ctx: CanvasRenderingContext2D,
  offsetX: number = 0,
  offsetY: number = 0,
): void {
  // Strip 0x prefix
  const hex = hash.startsWith('0x') ? hash.slice(2) : hash;

  // Parse nibbles (each hex char = one nibble, 0-15)
  const nibbles: number[] = [];
  for (let i = 0; i < Math.min(hex.length, 64); i++) {
    nibbles.push(parseInt(hex[i], 16));
  }

  if (nibbles.length === 0) return;

  // Total weight: sum of all nibble values (minimum 1 per nibble to avoid zero-width)
  const totalWeight = nibbles.reduce((sum, n) => sum + Math.max(1, n), 0);

  // Draw bars
  let x = offsetX;

  for (let i = 0; i < nibbles.length; i++) {
    const nibbleValue = nibbles[i];
    const barWidth = (Math.max(1, nibbleValue) / totalWeight) * width;

    // Two-tone pattern: even nibbles slightly brighter, odd nibbles slightly darker
    // Both are very subtle -- 5-15% opacity over the existing bar color
    const isEven = i % 2 === 0;

    if (nibbleValue > 8) {
      // High nibble: lighter band
      ctx.fillStyle = isEven
        ? `rgba(255, 255, 255, ${0.04 + (nibbleValue / 15) * 0.08})`
        : `rgba(255, 255, 255, ${0.02 + (nibbleValue / 15) * 0.06})`;
    } else {
      // Low nibble: darker band
      ctx.fillStyle = isEven
        ? `rgba(0, 0, 0, ${0.02 + ((15 - nibbleValue) / 15) * 0.08})`
        : `rgba(0, 0, 0, ${0.01 + ((15 - nibbleValue) / 15) * 0.06})`;
    }

    ctx.fillRect(x, offsetY, barWidth, height);
    x += barWidth;
  }
}

/**
 * Generate a standalone barcode image for use in tooltips or detail views.
 * Returns an ImageData that can be drawn with ctx.putImageData().
 */
export function hashToBarcodeImage(
  hash: string,
  width: number,
  height: number,
): ImageData {
  const offscreen = new OffscreenCanvas(width, height);
  const ctx = offscreen.getContext('2d')!;

  // Dark background
  ctx.fillStyle = '#12111a'; // --rd-void-surface
  ctx.fillRect(0, 0, width, height);

  hashToBarcode(hash, width, height, ctx);

  return ctx.getImageData(0, 0, width, height);
}
```

### Checklist

- [ ] `hashToBarcode(hash, width, height, ctx, offsetX, offsetY)` renders barcode inside the bar
- [ ] 64 nibbles from hash, each nibble controls bar width proportionally
- [ ] High nibbles (9-15): lighter bands (white overlay)
- [ ] Low nibbles (0-8): darker bands (black overlay)
- [ ] Alternating even/odd nibbles create two-tone visual rhythm
- [ ] Opacity range: 1-12% -- subtle, not overpowering the gas color
- [ ] `hashToBarcodeImage()` variant for standalone rendering (tooltip, detail view)
- [ ] Zero-width protection: `Math.max(1, nibbleValue)` prevents invisible bars

---

## 7.4 Scroll and Ring Buffer

The waterfall maintains a ring buffer of the last 128 blocks and provides smooth scroll interaction.

### Ring buffer

The block array in `WaterfallScene` acts as a ring buffer managed in `blocksRef.current`. Newest blocks are prepended (index 0 = newest). When the array exceeds `RING_SIZE` (128), the oldest block is popped. This reuses the same array -- no allocation churn.

```typescript
// In WaterfallScene.tsx -- onNewBlock callback (shown in 7.1)
// Prepend newest, pop oldest if over capacity.
// The chain store's LRU cache (500 blocks) holds the canonical data.
// The waterfall's 128-entry array is a view optimized for rendering.
```

### Smooth scroll animation

Scroll uses a lerp (linear interpolation) approach: the actual scroll position (`scrollOffsetRef`) chases the target (`targetScrollRef`) at a rate of `SCROLL_LERP` (0.12) per frame. This produces a smooth, decelerating motion regardless of input source (wheel, touch, programmatic).

```typescript
// Every frame in the render loop:
scrollOffsetRef.current += (targetScrollRef.current - scrollOffsetRef.current) * SCROLL_LERP;
```

At 60fps with lerp 0.12, the scroll reaches 95% of its target in ~24 frames (400ms). This feels responsive but not abrupt.

### Auto-scroll behavior

When auto-scroll is engaged (default), the target is always 0 (newest block at top). When a user scrolls down, auto-scroll disengages. It re-engages when the user scrolls back to within `AUTO_SCROLL_THRESHOLD` bars of the top.

```
User scrolls down:     autoScroll = false, target tracks user input
New block arrives:     target shifts by +rowHeight (keeps position stable)
User scrolls to top:   autoScroll = true, target snaps to 0
```

### Touch support

Touch drag mapping (for mobile and trackpad):

```typescript
// In WaterfallScene.tsx -- additional event handlers

const touchStartRef = useRef(0);
const touchScrollStartRef = useRef(0);

const onTouchStart = useCallback((e: TouchEvent) => {
  touchStartRef.current = e.touches[0].clientY;
  touchScrollStartRef.current = targetScrollRef.current;
}, []);

const onTouchMove = useCallback((e: TouchEvent) => {
  e.preventDefault();
  const deltaY = touchStartRef.current - e.touches[0].clientY;
  const rowHeight = BAR_HEIGHT + BAR_GAP;
  const maxScroll = Math.max(0, blocksRef.current.length * rowHeight - canvasHeight());

  targetScrollRef.current = Math.max(
    0,
    Math.min(maxScroll, touchScrollStartRef.current + deltaY),
  );

  autoScrollRef.current = targetScrollRef.current < AUTO_SCROLL_THRESHOLD * rowHeight;
}, []);
```

### Checklist

- [ ] Ring buffer: last 128 blocks, newest at index 0
- [ ] Oldest block evicted when buffer exceeds 128
- [ ] Smooth scroll: lerp factor 0.12, reaches 95% in ~400ms
- [ ] Auto-scroll: always show newest unless user scrolled down
- [ ] Auto-scroll re-engages when user scrolls within 2 bars of top
- [ ] New block arrival: auto-scroll -> target 0; manual scroll -> target shifts to keep position
- [ ] Mouse wheel: passive=false, preventDefault, clamped to valid range
- [ ] Touch drag: maps touchmove delta to scroll target
- [ ] Scroll indicator: 3px wide rose bar on right edge, visible when content overflows

---

## 7.5 Gas Waveform (Bottom Panel)

### File: `apps/explorer/src/scenes/waterfall/gasWaveform.ts`

A line chart below the waterfall showing `gasUsed` over the last 64 blocks, with a `baseFee` overlay line. Rendered on an OffscreenCanvas to avoid blocking the main thread.

```typescript
import { bus } from '../../bus/events';
import type { ChainBlock, FeeDataPoint } from '../../rpc/types';

/** Waveform configuration */
const WAVEFORM_HEIGHT = 120;      // px logical height
const POINT_COUNT = 64;           // blocks to show
const LINE_COLOR = '#aa7088';     // --rd-rose
const FILL_TOP = 'rgba(170, 112, 136, 0.10)';  // --rd-rose at 10%
const FILL_BOTTOM = 'rgba(170, 112, 136, 0.0)'; // transparent
const BASE_FEE_COLOR = '#c89a68'; // --rd-warning (burnt amber)
const GRID_COLOR = 'rgba(255, 255, 255, 0.03)';
const LABEL_COLOR = '#6a5a68';    // --rd-text-dim
const FONT = '9px "JetBrains Mono", monospace';

interface WaveformState {
  gasUsedHistory: number[];    // last 64 gasUsed values
  gasLimitHistory: number[];   // last 64 gasLimit values
  baseFeeHistory: number[];    // last 64 baseFee values (from feeHistory)
  blockNumbers: bigint[];      // corresponding block numbers
}

/**
 * GasWaveform component.
 *
 * Uses OffscreenCanvas if available, falls back to regular canvas.
 * Redraws only when new data arrives (not every frame).
 */
export class GasWaveformRenderer {
  private canvas: HTMLCanvasElement;
  private ctx: CanvasRenderingContext2D | OffscreenCanvasRenderingContext2D;
  private state: WaveformState = {
    gasUsedHistory: [],
    gasLimitHistory: [],
    baseFeeHistory: [],
    blockNumbers: [],
  };
  private width = 0;
  private height = WAVEFORM_HEIGHT;
  private dpr = 1;

  constructor(canvas: HTMLCanvasElement) {
    this.canvas = canvas;
    this.dpr = Math.min(window.devicePixelRatio || 1, 2);

    // Try OffscreenCanvas for worker-thread rendering
    if (typeof OffscreenCanvas !== 'undefined') {
      try {
        const offscreen = canvas.transferControlToOffscreen();
        this.ctx = offscreen.getContext('2d')!;
      } catch {
        // Fallback: regular context (OffscreenCanvas may not support transferControl)
        this.ctx = canvas.getContext('2d')!;
      }
    } else {
      this.ctx = canvas.getContext('2d')!;
    }
  }

  resize(width: number): void {
    this.width = width;
    this.canvas.width = width * this.dpr;
    this.canvas.height = this.height * this.dpr;
    this.canvas.style.width = `${width}px`;
    this.canvas.style.height = `${this.height}px`;
    this.ctx.setTransform(this.dpr, 0, 0, this.dpr, 0, 0);
    this.redraw();
  }

  /** Called when a new block arrives */
  pushBlock(block: ChainBlock): void {
    const s = this.state;

    s.gasUsedHistory.push(Number(block.gasUsed));
    s.gasLimitHistory.push(Number(block.gasLimit));
    s.blockNumbers.push(block.number);

    // Keep only last POINT_COUNT entries
    if (s.gasUsedHistory.length > POINT_COUNT) {
      s.gasUsedHistory.shift();
      s.gasLimitHistory.shift();
      s.blockNumbers.shift();
    }

    this.redraw();
  }

  /** Called when fee history updates (from eth_feeHistory poll) */
  updateFeeHistory(feeData: FeeDataPoint[]): void {
    this.state.baseFeeHistory = feeData
      .slice(-POINT_COUNT)
      .map((f) => Number(f.baseFee));
    this.redraw();
  }

  /** Full redraw of the waveform */
  private redraw(): void {
    const ctx = this.ctx;
    const w = this.width;
    const h = this.height;
    const s = this.state;

    if (w === 0 || s.gasUsedHistory.length < 2) return;

    ctx.clearRect(0, 0, w, h);

    // Margins
    const marginLeft = 48;   // space for Y-axis labels
    const marginRight = 12;
    const marginTop = 8;
    const marginBottom = 20; // space for X-axis labels
    const plotW = w - marginLeft - marginRight;
    const plotH = h - marginTop - marginBottom;

    // Y-axis range: 0 to max gasLimit (consistent ceiling)
    const maxGas = Math.max(...s.gasLimitHistory, 1);

    // Grid lines (4 horizontal)
    ctx.strokeStyle = GRID_COLOR;
    ctx.lineWidth = 1;
    for (let i = 0; i <= 4; i++) {
      const gy = marginTop + (plotH / 4) * i;
      ctx.beginPath();
      ctx.moveTo(marginLeft, gy);
      ctx.lineTo(marginLeft + plotW, gy);
      ctx.stroke();
    }

    // Compute points
    const points: [number, number][] = s.gasUsedHistory.map((gas, i) => {
      const x = marginLeft + (i / (POINT_COUNT - 1)) * plotW;
      const y = marginTop + plotH - (gas / maxGas) * plotH;
      return [x, y];
    });

    // Fill gradient (line to bottom)
    const gradient = ctx.createLinearGradient(0, marginTop, 0, marginTop + plotH);
    gradient.addColorStop(0, FILL_TOP);
    gradient.addColorStop(1, FILL_BOTTOM);

    ctx.beginPath();
    ctx.moveTo(points[0][0], marginTop + plotH); // start at bottom-left

    // Bezier-interpolated line (Catmull-Rom -> cubic bezier)
    for (let i = 0; i < points.length; i++) {
      if (i === 0) {
        ctx.lineTo(points[0][0], points[0][1]);
      } else {
        const p0 = points[Math.max(0, i - 2)];
        const p1 = points[i - 1];
        const p2 = points[i];
        const p3 = points[Math.min(points.length - 1, i + 1)];

        // Catmull-Rom to cubic bezier control points
        const cp1x = p1[0] + (p2[0] - p0[0]) / 6;
        const cp1y = p1[1] + (p2[1] - p0[1]) / 6;
        const cp2x = p2[0] - (p3[0] - p1[0]) / 6;
        const cp2y = p2[1] - (p3[1] - p1[1]) / 6;

        ctx.bezierCurveTo(cp1x, cp1y, cp2x, cp2y, p2[0], p2[1]);
      }
    }

    // Close the fill area
    ctx.lineTo(points[points.length - 1][0], marginTop + plotH);
    ctx.closePath();
    ctx.fillStyle = gradient;
    ctx.fill();

    // Stroke the line on top of the fill
    ctx.beginPath();
    for (let i = 0; i < points.length; i++) {
      if (i === 0) {
        ctx.moveTo(points[0][0], points[0][1]);
      } else {
        const p0 = points[Math.max(0, i - 2)];
        const p1 = points[i - 1];
        const p2 = points[i];
        const p3 = points[Math.min(points.length - 1, i + 1)];

        const cp1x = p1[0] + (p2[0] - p0[0]) / 6;
        const cp1y = p1[1] + (p2[1] - p0[1]) / 6;
        const cp2x = p2[0] - (p3[0] - p1[0]) / 6;
        const cp2y = p2[1] - (p3[1] - p1[1]) / 6;

        ctx.bezierCurveTo(cp1x, cp1y, cp2x, cp2y, p2[0], p2[1]);
      }
    }
    ctx.strokeStyle = LINE_COLOR;
    ctx.lineWidth = 1.5;
    ctx.stroke();

    // Base fee overlay line (if available)
    if (s.baseFeeHistory.length >= 2) {
      const maxFee = Math.max(...s.baseFeeHistory, 1);

      ctx.beginPath();
      s.baseFeeHistory.forEach((fee, i) => {
        const x = marginLeft + (i / (POINT_COUNT - 1)) * plotW;
        const y = marginTop + plotH - (fee / maxFee) * plotH;
        if (i === 0) ctx.moveTo(x, y);
        else ctx.lineTo(x, y);
      });
      ctx.strokeStyle = BASE_FEE_COLOR;
      ctx.lineWidth = 1;
      ctx.setLineDash([4, 4]);
      ctx.stroke();
      ctx.setLineDash([]); // reset
    }

    // Y-axis labels
    ctx.font = FONT;
    ctx.fillStyle = LABEL_COLOR;
    ctx.textAlign = 'right';
    ctx.textBaseline = 'middle';
    for (let i = 0; i <= 4; i++) {
      const gy = marginTop + (plotH / 4) * i;
      const value = maxGas * (1 - i / 4);
      ctx.fillText(formatGas(value), marginLeft - 6, gy);
    }

    // X-axis labels (block numbers, every ~16 blocks)
    ctx.textAlign = 'center';
    ctx.textBaseline = 'top';
    const step = Math.max(1, Math.floor(s.blockNumbers.length / 4));
    for (let i = 0; i < s.blockNumbers.length; i += step) {
      const x = marginLeft + (i / (POINT_COUNT - 1)) * plotW;
      ctx.fillText(
        Number(s.blockNumbers[i]).toLocaleString(),
        x,
        marginTop + plotH + 4,
      );
    }
  }
}

/** Format gas value for Y-axis labels */
function formatGas(value: number): string {
  if (value >= 1e9) return `${(value / 1e9).toFixed(1)}B`;
  if (value >= 1e6) return `${(value / 1e6).toFixed(1)}M`;
  if (value >= 1e3) return `${(value / 1e3).toFixed(0)}K`;
  return value.toFixed(0);
}
```

### React wrapper

```tsx
// apps/explorer/src/scenes/waterfall/GasWaveform.tsx

import { forwardRef, useEffect, useRef } from 'react';
import { bus } from '../../bus/events';
import { GasWaveformRenderer } from './gasWaveform';

export const GasWaveform = forwardRef<HTMLCanvasElement>((_props, ref) => {
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const rendererRef = useRef<GasWaveformRenderer | null>(null);

  useEffect(() => {
    const canvas = canvasRef.current;
    if (!canvas) return;

    const renderer = new GasWaveformRenderer(canvas);
    rendererRef.current = renderer;

    // Resize
    const observer = new ResizeObserver((entries) => {
      for (const entry of entries) {
        renderer.resize(entry.contentRect.width);
      }
    });
    observer.observe(canvas.parentElement!);

    // Subscribe to data
    const onBlock = (block: any) => renderer.pushBlock(block);
    const onFee = (data: any) => renderer.updateFeeHistory(data);
    bus.on('block:new', onBlock);
    bus.on('fee:update', onFee);

    return () => {
      bus.off('block:new', onBlock);
      bus.off('fee:update', onFee);
      observer.disconnect();
    };
  }, []);

  return (
    <canvas
      ref={canvasRef}
      style={{
        display: 'block',
        width: '100%',
        height: '120px',
        borderTop: '1px solid rgba(255, 255, 255, 0.04)',
      }}
    />
  );
});
```

### Checklist

- [ ] Separate canvas element, 120px tall, positioned below waterfall
- [ ] Line chart of `gasUsed` over last 64 blocks
- [ ] Fill gradient: `--rd-rose` at 10% opacity at top, transparent at bottom
- [ ] Smooth line: Catmull-Rom to cubic bezier interpolation between points
- [ ] Y-axis labels: gas values (auto-scaled: K/M/B suffixes)
- [ ] X-axis labels: block numbers at regular intervals
- [ ] Base fee overlay: dashed line in `--rd-warning` color (burnt amber)
- [ ] OffscreenCanvas used when available, fallback to regular canvas
- [ ] Redraws only on new data (not every frame)
- [ ] Grid lines: 4 horizontal lines at 3% white opacity
- [ ] ResizeObserver handles container width changes

---

## 7.6 Interaction

### Tooltip

```tsx
// apps/explorer/src/scenes/waterfall/tooltip.tsx

import { useState, useEffect, type MutableRefObject } from 'react';
import type { ChainBlock } from '../../rpc/types';

export interface TooltipData {
  block: ChainBlock;
  x: number;  // clientX
  y: number;  // clientY
  barY: number; // Y position of the bar in canvas
}

interface Props {
  setRef: MutableRefObject<(data: TooltipData | null) => void>;
}

export function WaterfallTooltip({ setRef }: Props) {
  const [data, setData] = useState<TooltipData | null>(null);

  useEffect(() => {
    setRef.current = setData;
  }, [setRef]);

  if (!data) return null;

  const { block, x, y } = data;
  const gasRatio = Number(block.gasUsed) / Number(block.gasLimit);
  const age = Date.now() - block._arrivalTime;
  const ageSec = (age / 1000).toFixed(1);

  return (
    <div
      style={{
        position: 'fixed',
        left: x + 12,
        top: y - 60,
        background: 'rgba(8, 8, 12, 0.85)',
        backdropFilter: 'blur(12px)',
        border: '1px solid rgba(255, 255, 255, 0.07)',
        padding: '8px 12px',
        fontFamily: '"JetBrains Mono", monospace',
        fontSize: '10px',
        color: '#c8b8c0',
        pointerEvents: 'none',
        zIndex: 200,
        lineHeight: 1.6,
        whiteSpace: 'nowrap',
      }}
    >
      <div style={{ color: '#dca5bd', marginBottom: 2 }}>
        BLOCK {Number(block.number).toLocaleString()}
      </div>
      <div style={{ color: '#6a5a68', letterSpacing: '0.06em' }}>
        {block.hash.slice(0, 18)}...{block.hash.slice(-8)}
      </div>
      <div>
        GAS {(gasRatio * 100).toFixed(1)}%
        <span style={{ color: '#6a5a68' }}> ({formatGasCompact(Number(block.gasUsed))} / {formatGasCompact(Number(block.gasLimit))})</span>
      </div>
      <div>
        {block.transactions.length} TXN
        <span style={{ color: '#6a5a68' }}> {ageSec}s ago</span>
      </div>
    </div>
  );
}

function formatGasCompact(gas: number): string {
  if (gas >= 1e9) return `${(gas / 1e9).toFixed(1)}B`;
  if (gas >= 1e6) return `${(gas / 1e6).toFixed(1)}M`;
  if (gas >= 1e3) return `${(gas / 1e3).toFixed(0)}K`;
  return gas.toString();
}
```

### Waveform hover

The waveform canvas also supports hover to show exact gas values at specific blocks. The hover handler maps the mouse X position to a block index in the history array and overlays a crosshair + label.

```typescript
// In GasWaveformRenderer -- add to redraw() or as a separate overlay
onMouseMove(clientX: number): { blockNumber: bigint; gasUsed: number; baseFee: number } | null {
  const s = this.state;
  if (s.gasUsedHistory.length < 2) return null;

  const marginLeft = 48;
  const marginRight = 12;
  const plotW = this.width - marginLeft - marginRight;
  const relX = clientX - marginLeft;

  if (relX < 0 || relX > plotW) return null;

  const index = Math.round((relX / plotW) * (s.gasUsedHistory.length - 1));
  if (index < 0 || index >= s.gasUsedHistory.length) return null;

  return {
    blockNumber: s.blockNumbers[index],
    gasUsed: s.gasUsedHistory[index],
    baseFee: s.baseFeeHistory[index] ?? 0,
  };
}
```

### Checklist

- [ ] Hover on block bar: tooltip appears with hash, timestamp, gas %, tx count
- [ ] Tooltip follows cursor, offset 12px right and 60px up
- [ ] Tooltip styled as glass panel (dark bg, blur, 7% white border)
- [ ] Click on block bar: emits `entity:select` -> opens BlockDetail panel
- [ ] Mouse leave: tooltip hides
- [ ] Hover on waveform: shows exact gas value at crosshair position
- [ ] Click behavior does not interfere with scroll gestures

---

## 7.7 Performance

### Frame budget

Target: **<2ms per frame** for the waterfall canvas.

| Operation | Budget |
|-----------|--------|
| Clear canvas | 0.05ms |
| Visible range calculation | 0.01ms |
| Block bars (50 visible) | 0.80ms (16us per bar) |
| Hash barcodes (50) | 0.40ms (8us per bar) |
| Labels (100 text draws) | 0.50ms |
| Scroll indicator | 0.02ms |
| **Total** | **~1.8ms** |

The waveform redraws only on new data (1/s for blocks, 1/5s for fees) and runs on OffscreenCanvas -- zero main-thread cost during idle frames.

### Dirty region optimization

Only redraw what changed:

```typescript
// Optimization: track which regions need redraw
interface DirtyRegion {
  type: 'new_block' | 'scroll' | 'hover' | 'full';
  y?: number;
  height?: number;
}

// On new block: dirty region = top bar + scroll shift (full redraw in practice,
//   because scroll shifts every bar's position)
// On scroll: full redraw (every bar position changed)
// On hover: redraw only the hovered bar and the previously hovered bar

// In practice, Canvas2D clearing and redrawing 128 bars at <2ms is fast enough
// that dirty-region tracking adds complexity without meaningful benefit.
// Only implement if profiling shows the frame budget is exceeded.
```

### Visibility skip

```typescript
// In render loop:
if (document.hidden) {
  rafIdRef.current = requestAnimationFrame(render);
  return; // skip all drawing, just keep the loop alive
}
```

When the tab is hidden, we keep `requestAnimationFrame` alive (so we resume immediately on tab focus) but skip all drawing work. The browser throttles rAF to ~1fps for hidden tabs anyway, so this is mostly about avoiding wasted GPU uploads.

### Memory

| Resource | Size | Notes |
|----------|------|-------|
| Block array (128 entries) | ~1.3MB | Full ChainBlock objects with transaction bodies |
| Canvas bitmap (main) | ~4MB | 1920x540 at 2x DPR = 3840x1080 RGBA |
| Canvas bitmap (waveform) | ~0.9MB | 1920x120 at 2x DPR |
| **Total** | **~6.2MB** | Well within budget |

### Checklist

- [ ] Canvas2D render: <2ms per frame (measure with `performance.now()`)
- [ ] OffscreenCanvas for waveform avoids main thread blocking
- [ ] Only visible bars drawn (viewport culling via `firstVisible`/`lastVisible`)
- [ ] Frame skipped when `document.hidden === true`
- [ ] No canvas operations when component is unmounted (`cancelAnimationFrame`)
- [ ] Memory: <7MB total for both canvases + block data
- [ ] No layout thrashing: canvas dimensions set once in ResizeObserver, not per frame

---

## 7.8 Verification

End-to-end acceptance criteria:

- [ ] **Bars render:** Blocks appear as horizontal bars with width proportional to `gasUsed / gasLimit`
- [ ] **Empty blocks:** Thin bars (4px minimum width), `void-surface` color, no tx label
- [ ] **Active blocks:** Wide bars, rose-toned, left accent, tx count label visible
- [ ] **Barcode visible:** Each bar shows a unique hash-derived barcode pattern inside
- [ ] **New block animation:** When a block arrives, it appears at the top and existing bars shift down smoothly
- [ ] **Scroll:** Mouse wheel and touch drag scroll through block history, smooth lerp motion
- [ ] **Auto-scroll:** Stays locked to newest block by default, disengages on user scroll, re-engages at top
- [ ] **Gas waveform:** Line chart updates with each new block, fill gradient visible
- [ ] **Base fee line:** Dashed amber line overlays the gas waveform (when feeHistory available)
- [ ] **Tooltip:** Hover on bar shows block hash, gas %, tx count, age
- [ ] **Navigation:** Click on bar emits `entity:select` -> BlockDetail opens
- [ ] **Waveform hover:** Crosshair on waveform shows exact gas value
- [ ] **60fps:** Solid 60fps with 128 blocks rendered, no jank during scroll
- [ ] **Retina:** Sharp rendering on 2x DPR displays
- [ ] **Resize:** Canvas adapts to container resize (ResizeObserver)
- [ ] **Tab hidden:** No drawing work when tab is not visible

---

## File Manifest

```
apps/explorer/src/scenes/waterfall/
├── WaterfallScene.tsx       # 7.1  React component, canvas refs, render loop, lifecycle
├── blockBar.ts              # 7.2  drawBlockBar(), gasToColor(), drawBlockBarHighlight()
├── hashbar.ts               # 7.3  hashToBarcode(), hashToBarcodeImage()
├── gasWaveform.ts           # 7.5  GasWaveformRenderer class (OffscreenCanvas)
├── GasWaveform.tsx          # 7.5  React wrapper for waveform canvas
└── tooltip.tsx              # 7.6  WaterfallTooltip component
```
