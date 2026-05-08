# ROSEDUST Design System -- Implementation Guide

Terminal-existentialist aesthetic. Monospace typography, low-contrast glass panels,
atmospheric fog, film grain. Every surface whispers; nothing shouts.

This document is the complete implementation reference for the ROSEDUST token system
as applied to the Kora Explorer. All values are final. Copy-paste into source files.

---

## File Map

```
apps/explorer/src/design/
  tokens.css            CSS custom properties on :root
  reset.css             Minimal reset, scrollbar, box-sizing
  typography.css        @font-face declarations, type scale utilities
  atmosphere.module.css Grain, vignette, scanlines, rose wash
  glass.module.css      Glass panel base + variants
```

All five files are loaded in `main.tsx` in this order:
```typescript
import './design/reset.css';
import './design/tokens.css';
import './design/typography.css';
import './design/atmosphere.module.css';
```
`glass.module.css` is imported per-component via CSS Modules.

---

## 1. CSS Custom Properties (tokens.css)

Single source of truth. Every visual value in the application references these tokens.

```css
/* apps/explorer/src/design/tokens.css */
:root {
  /* ================================================================
     COLORS -- VOID (backgrounds)
     ================================================================ */
  --rd-bg:                  hsl(220, 15%, 6%);        /* #0e0f13 -- deepest background */
  --rd-surface:             hsl(220, 12%, 10%);       /* #161821 -- card/panel fill */
  --rd-surface-raised:      hsl(220, 10%, 13%);       /* #1d1f28 -- elevated surface */
  --rd-void:                #060608;                   /* true black, scene bg */
  --rd-void-light:          #0c0c10;                  /* canvas clear color */
  --rd-void-surface:        #12111a;                  /* slightly lifted void */

  /* ================================================================
     COLORS -- ROSE (primary accent -- blocks, heartbeat, life)
     ================================================================ */
  --rd-rose-deep:           #3a2030;
  --rd-rose-dim:            #6a4058;
  --rd-rose:                #aa7088;
  --rd-rose-bright:         #cc90a8;
  --rd-rose-glow:           #dca5bd;

  /* ================================================================
     COLORS -- BONE (value, transactions, warmth)
     ================================================================ */
  --rd-bone-dim:            #8a7860;
  --rd-bone:                #b8a880;
  --rd-bone-bright:         #d8c8a0;

  /* ================================================================
     COLORS -- DREAM (pending, unresolved, liminal)
     ================================================================ */
  --rd-dream-dim:           #5a5a78;
  --rd-dream:               #7a7a98;
  --rd-dream-bright:        #9494b4;

  /* ================================================================
     COLORS -- GLASS (panel surfaces)
     ================================================================ */
  --rd-glass:               hsla(220, 15%, 12%, 0.6); /* semi-transparent panel bg */
  --rd-glass-bg:            rgba(8, 8, 12, 0.45);     /* default glass fill */
  --rd-glass-bg-hover:      rgba(170, 112, 136, 0.08);/* rose-tinted hover */
  --rd-glass-highlight:     rgba(255, 255, 255, 0.06);/* inset top-edge light */
  --rd-glass-blur:          12px;
  --rd-glass-saturate:      180%;

  /* ================================================================
     COLORS -- TEXT
     ================================================================ */
  --rd-text:                hsl(45, 10%, 82%);        /* #d4cfc5 -- primary body */
  --rd-text-dim:            hsl(45, 8%, 55%);         /* #908d82 -- secondary/labels */
  --rd-text-ghost:          #3a303a;                  /* structural, barely visible */
  --rd-text-bright:         #e8d8e0;                  /* emphasis, active elements */

  /* ================================================================
     COLORS -- ACCENT
     ================================================================ */
  --rd-accent:              hsl(12, 70%, 55%);        /* #d05a3a -- call-to-action, focus */
  --rd-accent-dim:          hsla(12, 70%, 55%, 0.3);  /* accent at low intensity */

  /* ================================================================
     COLORS -- SEMANTIC
     ================================================================ */
  --rd-success:             #7a8a78;                  /* sage green */
  --rd-warning:             #c89a68;                  /* burnt amber */
  --rd-error:               #cc5555;                  /* coral red, used sparingly */
  --rd-danger:              #cc5555;                  /* alias for error */

  /* ================================================================
     COLORS -- CONSENSUS PHASES
     ================================================================ */
  --rd-consensus-propose:   var(--rd-bone-bright);    /* proposal = warm bone */
  --rd-consensus-vote:      var(--rd-dream-bright);   /* voting = liminal dream */
  --rd-consensus-finalize:  var(--rd-rose-glow);      /* finalized = bright rose */
  --rd-consensus-nullify:   var(--rd-text-ghost);     /* nullified = faded, empty */

  /* ================================================================
     COLORS -- BORDERS
     ================================================================ */
  --rd-border:              rgba(255, 255, 255, 0.07);
  --rd-border-strong:       rgba(255, 255, 255, 0.14);
  --rd-border-rose:         rgba(170, 112, 136, 0.3);
  --rd-panel-border:        1px solid hsla(45, 10%, 82%, 0.08);

  /* ================================================================
     COLORS -- LED INDICATORS
     ================================================================ */
  --rd-led-size:            5px;
  --rd-led-connected:       var(--rd-success);
  --rd-led-warning:         var(--rd-warning);
  --rd-led-error:           var(--rd-error);
  --rd-led-streaming:       var(--rd-rose-glow);

  /* ================================================================
     TYPOGRAPHY
     ================================================================ */
  --rd-font-mono:           'Berkeley Mono', 'JetBrains Mono', 'SF Mono', monospace;
  --rd-font-display:        'Fraunces', Georgia, serif;

  --rd-font-size-xs:        10px;
  --rd-font-size-sm:        11px;
  --rd-font-size-base:      14px;
  --rd-font-size-lg:        18px;
  --rd-font-size-xl:        24px;
  --rd-font-size-2xl:       32px;
  --rd-font-size-3xl:       38px;    /* hero block number */

  --rd-line-height:         1.5;
  --rd-line-height-tight:   1.2;
  --rd-line-height-mono:    1.6;

  --rd-letter-spacing:      0.06em;  /* default mono tracking */
  --rd-tracking-tight:      0.02em;
  --rd-tracking-normal:     0.06em;
  --rd-tracking-wide:       0.12em;
  --rd-tracking-label:      0.28em;  /* address labels, panel headers */
  --rd-tracking-section:    0.32em;  /* section tags: -- 01 . BLOCKS */

  /* ================================================================
     SPACING (4px base grid)
     ================================================================ */
  --rd-space-1:             4px;
  --rd-space-2:             8px;
  --rd-space-3:             12px;
  --rd-space-4:             16px;
  --rd-space-5:             20px;
  --rd-space-6:             24px;
  --rd-space-7:             28px;
  --rd-space-8:             32px;
  --rd-space-9:             36px;
  --rd-space-10:            40px;
  --rd-space-11:            44px;
  --rd-space-12:            48px;

  /* Named aliases for readability */
  --rd-space-xs:            var(--rd-space-1);   /* 4px */
  --rd-space-sm:            var(--rd-space-2);   /* 8px */
  --rd-space-md:            var(--rd-space-4);   /* 16px */
  --rd-space-lg:            var(--rd-space-6);   /* 24px */
  --rd-space-xl:            var(--rd-space-8);   /* 32px */
  --rd-space-2xl:           var(--rd-space-12);  /* 48px */

  /* ================================================================
     LAYOUT
     ================================================================ */
  --rd-panel-radius:        2px;     /* near-sharp, not fully zero */
  --rd-panel-backdrop:      blur(12px) saturate(1.4);
  --rd-panel-max-width:     480px;
  --rd-panel-min-width:     280px;

  /* ================================================================
     ANIMATION / TRANSITIONS
     ================================================================ */
  --rd-ease-out:            cubic-bezier(0.16, 1, 0.3, 1);     /* expo ease-out */
  --rd-ease-in-out:         cubic-bezier(0.65, 0, 0.35, 1);
  --rd-ease-spring:         cubic-bezier(0.34, 1.56, 0.64, 1); /* overshoot */
  --rd-duration-fast:       120ms;
  --rd-duration-normal:     240ms;
  --rd-duration-slow:       600ms;
  --rd-duration-glacial:    1200ms;  /* ambient mode transitions */

  /* ================================================================
     Z-INDEX LAYERS
     ================================================================ */
  --rd-z-scene:             1;
  --rd-z-panels:            100;
  --rd-z-overlay:           500;
  --rd-z-search:            600;
  --rd-z-detail:            700;
  --rd-z-grain:             9997;
  --rd-z-vignette:          9998;
  --rd-z-scanlines:         9999;

  /* ================================================================
     ATMOSPHERE
     ================================================================ */
  --rd-fog-near:            0.0;     /* fog start distance (normalized) */
  --rd-fog-far:             1.0;     /* fog end distance (normalized) */
  --rd-fog-color:           var(--rd-void);
  --rd-grain-opacity:       0.035;   /* fractal noise overlay */
  --rd-grain-frequency:     0.85;    /* feTurbulence baseFrequency */
  --rd-vignette-opacity:    0.72;    /* edge darkening strength */
  --rd-vignette-center:     50% 30%; /* focal point (slightly above center) */
  --rd-scanline-opacity:    0.06;    /* CRT scanline overlay */
  --rd-scanline-gap:        2px;     /* transparent band */
  --rd-scanline-width:      1px;     /* dark band */

  /* ================================================================
     SCENE-SPECIFIC (driven by JS at runtime)
     ================================================================ */
  --rd-activity:            0;       /* 0..0.15, set by JS on new blocks */
  --rd-ambient-opacity:     1;       /* fades to 0.2 in ambient/idle mode */
}
```

**Token count: 72 custom properties.**

### Implementation Checklist -- tokens.css

- [ ] File created at `apps/explorer/src/design/tokens.css`
- [ ] All 72 tokens defined on `:root`
- [ ] `--rd-bg` resolves to `hsl(220, 15%, 6%)` via `getComputedStyle(document.documentElement).getPropertyValue('--rd-bg')`
- [ ] `--rd-font-mono` includes Berkeley Mono as first choice, JetBrains Mono as fallback
- [ ] `--rd-space-1` through `--rd-space-12` follow 4px increments (4, 8, 12, ... 48)
- [ ] `--rd-panel-border` is a complete `border` shorthand (1px solid hsla(...))
- [ ] `--rd-panel-backdrop` is a complete `backdrop-filter` value (blur + saturate)
- [ ] `--rd-consensus-*` tokens reference other tokens via `var()`
- [ ] `--rd-activity` defaults to `0`, writable by JS via `document.documentElement.style.setProperty`
- [ ] No `rem` or `em` units in spacing tokens (all `px` for pixel-precise control)
- [ ] Imported in `main.tsx` after `reset.css`, before `typography.css`

---

## 2. Glass Panel CSS Module (glass.module.css)

All data overlays use the glass panel system. Sharp corners. Translucent backdrop.
Asymmetric hover timing (fast in, slow out).

```css
/* apps/explorer/src/design/glass.module.css */

/* ================================================================
   BASE GLASS PANEL
   ================================================================ */
.glass {
  background: var(--rd-glass-bg);
  backdrop-filter: blur(var(--rd-glass-blur)) saturate(var(--rd-glass-saturate));
  -webkit-backdrop-filter: blur(var(--rd-glass-blur)) saturate(var(--rd-glass-saturate));
  border: var(--rd-panel-border);
  border-radius: var(--rd-panel-radius);
  box-shadow: inset 0 1px 0 var(--rd-glass-highlight);
  position: relative;
  color: var(--rd-text);
  font-family: var(--rd-font-mono);
  font-size: var(--rd-font-size-base);
  padding: var(--rd-space-md);

  /* Default transition: slow return from hover */
  transition:
    transform 120ms var(--rd-ease-out),
    background var(--rd-duration-normal) var(--rd-ease-out),
    box-shadow var(--rd-duration-normal) var(--rd-ease-out),
    opacity var(--rd-duration-normal) var(--rd-ease-out);
}

/* ================================================================
   VARIANT: ELEVATED (floating panels, search overlay, modals)
   ================================================================ */
.glassElevated {
  composes: glass;
  background: rgba(12, 12, 18, 0.65);
  box-shadow:
    inset 0 1px 0 var(--rd-glass-highlight),
    0 8px 32px rgba(0, 0, 0, 0.4),
    0 2px 8px rgba(0, 0, 0, 0.2);
  border-color: var(--rd-border-strong);
}

/* ================================================================
   VARIANT: SUNKEN (embedded panels, secondary info)
   ================================================================ */
.glassSunken {
  composes: glass;
  background: rgba(4, 4, 8, 0.35);
  box-shadow: inset 0 2px 6px rgba(0, 0, 0, 0.3);
  border-color: rgba(255, 255, 255, 0.04);
}

/* ================================================================
   VARIANT: INTERACTIVE (clickable panels, block tiles, tx rows)
   ================================================================ */
.glassInteractive {
  composes: glass;
  cursor: pointer;
  user-select: none;
}

/* Asymmetric hover: fast enter */
.glassInteractive:hover {
  transform: translateY(-2px) translateX(1px);
  background: var(--rd-glass-bg-hover);
  box-shadow:
    inset 0 1px 0 var(--rd-glass-highlight),
    0 4px 16px rgba(170, 112, 136, 0.08);
  transition-duration: var(--rd-duration-fast), var(--rd-duration-fast), var(--rd-duration-fast), var(--rd-duration-fast);
}

/* Asymmetric hover: slow leave (already set by base .glass transition) */

.glassInteractive:active {
  transform: translateY(0px) translateX(0px);
  transition-duration: 60ms;
}

/* ================================================================
   FOCUS RING (keyboard navigation)
   ================================================================ */
.glass:focus-visible {
  outline: 2px solid var(--rd-accent);
  outline-offset: 2px;
}

.glassInteractive:focus-visible {
  outline: 2px solid var(--rd-accent);
  outline-offset: 2px;
  box-shadow:
    inset 0 1px 0 var(--rd-glass-highlight),
    0 0 0 4px var(--rd-accent-dim);
}

/* ================================================================
   PANEL HEADER (label bar with LED indicator)
   ================================================================ */
.panelHeader {
  display: flex;
  align-items: center;
  gap: var(--rd-space-2);
  padding-bottom: var(--rd-space-3);
  margin-bottom: var(--rd-space-3);
  border-bottom: 1px solid rgba(255, 255, 255, 0.04);
}

.panelLabel {
  font-family: var(--rd-font-mono);
  font-size: var(--rd-font-size-xs);
  text-transform: uppercase;
  letter-spacing: var(--rd-tracking-label);
  color: var(--rd-text-dim);
  line-height: 1;
}

/* ================================================================
   ACTIVE PANEL (left rose accent border for live data)
   ================================================================ */
.panelActive {
  border-left: 2px solid var(--rd-rose-dim);
}

/* ================================================================
   LED STATUS DOT
   ================================================================ */
.led {
  width: var(--rd-led-size);
  height: var(--rd-led-size);
  border-radius: 50%;
  flex-shrink: 0;
}

.ledConnected {
  composes: led;
  background: var(--rd-led-connected);
  box-shadow: 0 0 6px var(--rd-led-connected);
  animation: ledPulse 2.4s ease-in-out infinite;
}

.ledWarning {
  composes: led;
  background: var(--rd-led-warning);
  box-shadow: 0 0 6px var(--rd-led-warning);
  animation: ledPulse 1.2s ease-in-out infinite;
}

.ledError {
  composes: led;
  background: var(--rd-led-error);
  box-shadow: 0 0 8px var(--rd-led-error);
  animation: ledPulse 0.8s ease-in-out infinite;
}

.ledStreaming {
  composes: led;
  background: var(--rd-led-streaming);
  box-shadow: 0 0 6px var(--rd-led-streaming);
  animation: ledPulse 1.6s ease-in-out infinite;
}

@keyframes ledPulse {
  0%, 100% { opacity: 0.7; }
  50%      { opacity: 1.0; }
}

/* ================================================================
   RESPONSIVE ADAPTATIONS
   ================================================================ */

/* Large desktop: panels at fixed positions */
@media (min-width: 1400px) {
  .glass {
    max-width: var(--rd-panel-max-width);
  }
}

/* Medium desktop: panels allowed to stretch */
@media (min-width: 1100px) and (max-width: 1399px) {
  .glass {
    max-width: 100%;
  }
}

/* Tablet: panels go full-width, reduce padding */
@media (min-width: 760px) and (max-width: 1099px) {
  .glass {
    max-width: 100%;
    padding: var(--rd-space-3);
    border-radius: 0;
  }

  .glassElevated {
    box-shadow:
      inset 0 1px 0 var(--rd-glass-highlight),
      0 4px 16px rgba(0, 0, 0, 0.3);
  }
}

/* Mobile: full-width panels, no hover effects, thinner borders */
@media (max-width: 759px) {
  .glass {
    max-width: 100%;
    padding: var(--rd-space-2) var(--rd-space-3);
    border-radius: 0;
    border-left: none;
    border-right: none;
  }

  .glassInteractive:hover {
    transform: none;
    background: var(--rd-glass-bg);
  }

  .glassInteractive:active {
    background: var(--rd-glass-bg-hover);
  }

  .panelActive {
    border-left: none;
    border-top: 2px solid var(--rd-rose-dim);
  }
}
```

### Implementation Checklist -- glass.module.css

- [ ] File created at `apps/explorer/src/design/glass.module.css`
- [ ] `.glass` base class applies `backdrop-filter: blur(12px) saturate(180%)`
- [ ] `-webkit-backdrop-filter` prefix present for Safari
- [ ] `.glassElevated` adds `box-shadow` with two shadow layers (ambient + drop)
- [ ] `.glassSunken` uses `inset` box-shadow and lower-opacity background
- [ ] `.glassInteractive` hover translates by `(-2px, 1px)` (asymmetric: up and right)
- [ ] Hover-in duration is `--rd-duration-fast` (120ms), hover-out is `--rd-duration-normal` (240ms)
- [ ] `:active` state snaps back to `translate(0, 0)` in 60ms
- [ ] `:focus-visible` shows `--rd-accent` outline with 2px offset
- [ ] `.panelActive` has 2px left border in `--rd-rose-dim`
- [ ] `.led` variants use distinct pulse speeds: connected=2.4s, warning=1.2s, error=0.8s, streaming=1.6s
- [ ] Border-radius is `2px` (near-zero, not fully rounded)
- [ ] At `<760px` viewport, hover transforms are disabled (touch devices)
- [ ] At `<760px` viewport, `.panelActive` accent moves from left border to top border
- [ ] CSS Modules generate scoped class names (verify no global class leaks)

---

## 3. Atmosphere CSS (atmosphere.module.css)

Four layers that sit above all content. They transform a flat UI into a physical space.
All layers use `pointer-events: none` and fixed positioning. They never interfere with interaction.

```css
/* apps/explorer/src/design/atmosphere.module.css */

/* ================================================================
   GRAIN OVERLAY (z-index 9997)

   Fractal noise at low opacity. Gives every surface subtle analog
   texture. The eye reads it as "physical" even at 3.5%.

   Technique: inline SVG feTurbulence as CSS background-image.
   This avoids an extra network request and renders at GPU speed.
   ================================================================ */
.grain {
  position: fixed;
  inset: 0;
  pointer-events: none;
  z-index: var(--rd-z-grain);
  opacity: var(--rd-grain-opacity);
  mix-blend-mode: overlay;
  background-image: url("data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' width='300' height='300'%3E%3Cfilter id='grain'%3E%3CfeTurbulence type='fractalNoise' baseFrequency='0.85' numOctaves='3' stitchTiles='stitch'/%3E%3C/filter%3E%3Crect width='100%25' height='100%25' filter='url(%23grain)'/%3E%3C/svg%3E");
  background-repeat: repeat;

  /* Subtle animation: shift the grain pattern to prevent static-looking noise */
  animation: grainShift 8s steps(10) infinite;
}

@keyframes grainShift {
  0%   { transform: translate(0, 0); }
  10%  { transform: translate(-5%, -5%); }
  20%  { transform: translate(2%, 3%); }
  30%  { transform: translate(-3%, 1%); }
  40%  { transform: translate(4%, -2%); }
  50%  { transform: translate(-1%, 4%); }
  60%  { transform: translate(3%, -3%); }
  70%  { transform: translate(-4%, 2%); }
  80%  { transform: translate(1%, -1%); }
  90%  { transform: translate(-2%, 3%); }
  100% { transform: translate(0, 0); }
}

/* ================================================================
   VIGNETTE (z-index 9998)

   Radial gradient that darkens edges. Focal point is slightly above
   center (50% 30%) to frame content with cinematic gravity.
   ================================================================ */
.vignette {
  position: fixed;
  inset: 0;
  pointer-events: none;
  z-index: var(--rd-z-vignette);
  background: radial-gradient(
    ellipse at var(--rd-vignette-center),
    transparent 50%,
    rgba(6, 6, 8, var(--rd-vignette-opacity)) 100%
  );
}

/* ================================================================
   SCANLINES (z-index 9999)

   CRT artifact. 2px transparent band, 1px dark band, repeating.
   At 6% opacity it is subliminal. You feel it more than see it.
   ================================================================ */
.scanlines {
  position: fixed;
  inset: 0;
  pointer-events: none;
  z-index: var(--rd-z-scanlines);
  background: repeating-linear-gradient(
    to bottom,
    transparent 0,
    transparent var(--rd-scanline-gap),
    rgba(0, 0, 0, 0.45) var(--rd-scanline-gap),
    rgba(0, 0, 0, 0.45) calc(var(--rd-scanline-gap) + var(--rd-scanline-width))
  );
  opacity: var(--rd-scanline-opacity);
  mix-blend-mode: multiply;
}

/* ================================================================
   FOG GRADIENT LAYERS

   Two fog layers create depth in the scene. Near-fog fades the
   bottom edge; far-fog fades the top. Together they create a
   vertical depth cue that grounds the 3D terrain.
   ================================================================ */
.fogNear {
  position: fixed;
  bottom: 0;
  left: 0;
  right: 0;
  height: 30vh;
  pointer-events: none;
  z-index: calc(var(--rd-z-panels) - 2);
  background: linear-gradient(
    to top,
    var(--rd-fog-color) 0%,
    transparent 100%
  );
  opacity: 0.8;
}

.fogFar {
  position: fixed;
  top: 0;
  left: 0;
  right: 0;
  height: 20vh;
  pointer-events: none;
  z-index: calc(var(--rd-z-panels) - 2);
  background: linear-gradient(
    to bottom,
    var(--rd-fog-color) 0%,
    transparent 100%
  );
  opacity: 0.5;
}

/* ================================================================
   ROSE WASH

   Subtle radial glow that tracks chain activity. Empty chain =
   invisible. Active blocks = faint rose light from center.
   --rd-activity is set by JS: range 0 (idle) to 0.15 (active).
   The room breathes with chain activity.
   ================================================================ */
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

/* ================================================================
   REDUCED MOTION

   Respect prefers-reduced-motion:
   - Disable grain animation entirely
   - Reduce all transition durations to near-instant
   - Remove rose wash pulsing
   - Keep static vignette and scanlines (they are not animated)
   ================================================================ */
@media (prefers-reduced-motion: reduce) {
  .grain {
    animation: none;
    opacity: 0.02;    /* still present, just less intense */
  }

  .roseWash {
    transition-duration: 0ms;
  }
}
```

### React Component for Atmosphere Layers

The atmosphere layers are rendered once in `App.tsx`:

```tsx
// apps/explorer/src/App.tsx (excerpt)
import styles from './design/atmosphere.module.css';

function AtmosphereOverlay() {
  return (
    <>
      <div className={styles.fogNear} />
      <div className={styles.fogFar} />
      <div className={styles.roseWash} />
      <div className={styles.grain} />
      <div className={styles.vignette} />
      <div className={styles.scanlines} />
    </>
  );
}
```

### Implementation Checklist -- atmosphere.module.css

- [ ] File created at `apps/explorer/src/design/atmosphere.module.css`
- [ ] `.grain` uses inline SVG `feTurbulence` as `background-image` (no external file)
- [ ] `.grain` opacity is `0.035` (var reference to `--rd-grain-opacity`)
- [ ] `.grain` `mix-blend-mode` is `overlay`
- [ ] `.grain` animates via `grainShift` keyframes using `steps(10)` to avoid smooth interpolation
- [ ] `.vignette` gradient center is at `50% 30%` (above center)
- [ ] `.vignette` edge opacity reaches `0.72` (var reference to `--rd-vignette-opacity`)
- [ ] `.scanlines` uses `repeating-linear-gradient` with 2px gap and 1px dark band
- [ ] `.scanlines` opacity is `0.06` with `mix-blend-mode: multiply`
- [ ] `.fogNear` covers bottom 30vh, fades upward from `--rd-fog-color`
- [ ] `.fogFar` covers top 20vh, fades downward from `--rd-fog-color`
- [ ] `.roseWash` references `--rd-activity` CSS variable for dynamic intensity
- [ ] `.roseWash` transitions over 2s ease-out (not instant)
- [ ] All atmosphere layers use `pointer-events: none` (verify no click interception)
- [ ] All atmosphere layers use `position: fixed` with `inset: 0`
- [ ] Z-index ordering: fogNear/fogFar (98) < roseWash (99) < grain (9997) < vignette (9998) < scanlines (9999)
- [ ] `@media (prefers-reduced-motion: reduce)` disables grain animation
- [ ] `@media (prefers-reduced-motion: reduce)` sets grain opacity to `0.02`
- [ ] `@media (prefers-reduced-motion: reduce)` sets roseWash transition to `0ms`
- [ ] Grain renders at 60fps in Chrome DevTools Performance tab (no jank)
- [ ] Six `<div>` elements rendered in `AtmosphereOverlay` component

---

## 4. Font Loading

### @font-face Declarations (typography.css)

Berkeley Mono is the preferred font. JetBrains Mono is the public fallback.
Fraunces is used for hero display numbers (block number in italic).

```css
/* apps/explorer/src/design/typography.css */

/* ================================================================
   BERKELEY MONO (preferred -- requires license)

   If Berkeley Mono is not available, the font-face silently fails
   and --rd-font-mono falls through to JetBrains Mono.
   ================================================================ */
@font-face {
  font-family: 'Berkeley Mono';
  src: url('/fonts/BerkeleyMono-Regular.woff2') format('woff2');
  font-weight: 400;
  font-style: normal;
  font-display: swap;
}

@font-face {
  font-family: 'Berkeley Mono';
  src: url('/fonts/BerkeleyMono-Bold.woff2') format('woff2');
  font-weight: 700;
  font-style: normal;
  font-display: swap;
}

/* ================================================================
   JETBRAINS MONO (fallback -- open source, variable font)
   ================================================================ */
@font-face {
  font-family: 'JetBrains Mono';
  src: url('/fonts/JetBrainsMono-Variable.woff2') format('woff2');
  font-style: normal;
  font-display: swap;
  font-weight: 100 800;
}

/* ================================================================
   FRAUNCES (display -- hero block numbers, italic only)
   ================================================================ */
@font-face {
  font-family: 'Fraunces';
  src: url('/fonts/Fraunces-Italic-Variable.woff2') format('woff2');
  font-style: italic;
  font-display: swap;
  font-weight: 100 900;
}

/* ================================================================
   TYPE SCALE UTILITIES
   ================================================================ */

/* Section tags: -- 01 . BLOCKS */
.sectionTag {
  font-family: var(--rd-font-mono);
  font-size: var(--rd-font-size-sm);
  letter-spacing: var(--rd-tracking-section);
  text-transform: uppercase;
  color: var(--rd-text-dim);
}

/* Address labels */
.addressLabel {
  font-family: var(--rd-font-mono);
  font-size: var(--rd-font-size-xs);
  letter-spacing: var(--rd-tracking-label);
  text-transform: uppercase;
  color: var(--rd-text-dim);
}

/* Hash display (truncated, dim) */
.hashText {
  font-family: var(--rd-font-mono);
  font-size: var(--rd-font-size-sm);
  letter-spacing: var(--rd-tracking-normal);
  color: var(--rd-text-dim);
}

/* Hero block number */
.heroNumber {
  font-family: var(--rd-font-display);
  font-size: var(--rd-font-size-3xl);
  font-style: italic;
  font-weight: 300;
  color: var(--rd-bone-bright);
  line-height: var(--rd-line-height-tight);
}

/* Transaction value */
.valueText {
  font-family: var(--rd-font-mono);
  font-size: var(--rd-font-size-base);
  color: var(--rd-bone-bright);
}

/* Status text with LED prefix */
.statusText {
  font-family: var(--rd-font-mono);
  font-size: var(--rd-font-size-xs);
  text-transform: uppercase;
  letter-spacing: var(--rd-tracking-label);
  color: var(--rd-text-dim);
}
```

### Preload Links (index.html)

Add to `<head>` in `apps/explorer/index.html`:

```html
<!-- Font preloads -- critical for avoiding FOIT -->
<link rel="preload" href="/fonts/JetBrainsMono-Variable.woff2" as="font" type="font/woff2" crossorigin>
<link rel="preload" href="/fonts/Fraunces-Italic-Variable.woff2" as="font" type="font/woff2" crossorigin>
<!-- Berkeley Mono preload only if licensed and file exists -->
<!-- <link rel="preload" href="/fonts/BerkeleyMono-Regular.woff2" as="font" type="font/woff2" crossorigin> -->
```

### Font File Inventory

```
apps/explorer/public/fonts/
  BerkeleyMono-Regular.woff2     (~25KB, optional, licensed)
  BerkeleyMono-Bold.woff2        (~25KB, optional, licensed)
  JetBrainsMono-Variable.woff2   (~55KB, required, OFL license)
  Fraunces-Italic-Variable.woff2 (~45KB, required, OFL license)
```

Total font payload: ~100KB (JetBrains + Fraunces), ~150KB if Berkeley Mono is included.

### Implementation Checklist -- Font Loading

- [ ] `typography.css` created at `apps/explorer/src/design/typography.css`
- [ ] Berkeley Mono `@font-face` declares regular (400) and bold (700) weights
- [ ] JetBrains Mono `@font-face` uses variable font range `100 800`
- [ ] Fraunces `@font-face` uses variable font range `100 900`, italic style
- [ ] All `@font-face` rules use `font-display: swap`
- [ ] `index.html` contains `<link rel="preload">` for JetBrainsMono-Variable.woff2
- [ ] `index.html` contains `<link rel="preload">` for Fraunces-Italic-Variable.woff2
- [ ] Preload links include `crossorigin` attribute (required for font preloads)
- [ ] Font files exist in `apps/explorer/public/fonts/` directory
- [ ] JetBrainsMono-Variable.woff2 is present (required)
- [ ] Fraunces-Italic-Variable.woff2 is present (required)
- [ ] If Berkeley Mono is absent, text renders in JetBrains Mono without layout shift
- [ ] No FOIT (Flash of Invisible Text) -- text appears immediately in fallback
- [ ] `.heroNumber` class renders in Fraunces italic at 38px
- [ ] `.addressLabel` class renders in mono at 10px with 0.28em tracking

---

## 5. Reset CSS (reset.css)

Minimal reset. No normalize.css. Just the essentials for the ROSEDUST aesthetic.

```css
/* apps/explorer/src/design/reset.css */

/* ================================================================
   BOX MODEL
   ================================================================ */
*,
*::before,
*::after {
  box-sizing: border-box;
  margin: 0;
  padding: 0;
}

/* ================================================================
   DOCUMENT
   ================================================================ */
html {
  -webkit-font-smoothing: antialiased;
  -moz-osx-font-smoothing: grayscale;
  text-rendering: optimizeLegibility;
  color-scheme: dark;
}

body {
  min-height: 100vh;
  min-height: 100dvh;
  background-color: var(--rd-void);
  color: var(--rd-text);
  font-family: var(--rd-font-mono);
  font-size: var(--rd-font-size-base);
  line-height: var(--rd-line-height);
  letter-spacing: var(--rd-letter-spacing);
  overflow-x: hidden;
}

/* ================================================================
   SCROLLBAR STYLING (Chromium + Firefox)
   ================================================================ */

/* Chromium (Chrome, Edge, Brave) */
::-webkit-scrollbar {
  width: 6px;
  height: 6px;
}

::-webkit-scrollbar-track {
  background: transparent;
}

::-webkit-scrollbar-thumb {
  background: var(--rd-text-ghost);
  border-radius: 3px;
}

::-webkit-scrollbar-thumb:hover {
  background: var(--rd-text-dim);
}

/* Firefox */
* {
  scrollbar-width: thin;
  scrollbar-color: var(--rd-text-ghost) transparent;
}

/* ================================================================
   TYPOGRAPHY RESETS
   ================================================================ */
h1, h2, h3, h4, h5, h6 {
  font-size: inherit;
  font-weight: inherit;
}

a {
  color: inherit;
  text-decoration: none;
}

/* ================================================================
   MEDIA RESETS
   ================================================================ */
img, svg, video, canvas {
  display: block;
  max-width: 100%;
}

/* ================================================================
   INTERACTIVE RESETS
   ================================================================ */
button {
  font: inherit;
  color: inherit;
  background: none;
  border: none;
  cursor: pointer;
}

input, textarea, select {
  font: inherit;
  color: inherit;
  background: none;
  border: none;
}

/* Remove default focus outline (replaced by ROSEDUST focus ring in glass.module.css) */
:focus:not(:focus-visible) {
  outline: none;
}

/* ================================================================
   SELECTION COLOR
   ================================================================ */
::selection {
  background: var(--rd-rose-dim);
  color: var(--rd-text-bright);
}
```

### Implementation Checklist -- reset.css

- [ ] File created at `apps/explorer/src/design/reset.css`
- [ ] `box-sizing: border-box` applied to all elements, `::before`, and `::after`
- [ ] `body` background is `--rd-void` (#060608)
- [ ] `body` font is `--rd-font-mono` at `--rd-font-size-base` (14px)
- [ ] `body` uses `100dvh` min-height (dynamic viewport for mobile)
- [ ] `-webkit-font-smoothing: antialiased` set on `html`
- [ ] Scrollbar width is 6px in Chromium, `thin` in Firefox
- [ ] Scrollbar thumb color is `--rd-text-ghost`, hover is `--rd-text-dim`
- [ ] Heading elements (`h1`-`h6`) reset to `inherit` size and weight
- [ ] `button` resets background, border, and inherits font
- [ ] `::selection` uses `--rd-rose-dim` background with `--rd-text-bright` text
- [ ] `:focus:not(:focus-visible)` removes outline for mouse clicks (keyboard focus preserved)
- [ ] `overflow-x: hidden` on body (prevent horizontal scroll from scene elements)
- [ ] Loaded FIRST in `main.tsx` import order (before tokens.css)

---

## 6. Full Verification Checklist

Run these checks after all five files are in place and the app compiles.

### Token Accessibility

- [ ] Open DevTools, run: `getComputedStyle(document.documentElement).getPropertyValue('--rd-bg')` -- returns `hsl(220, 15%, 6%)`
- [ ] Run: `getComputedStyle(document.documentElement).getPropertyValue('--rd-font-mono')` -- returns string containing `Berkeley Mono` or `JetBrains Mono`
- [ ] Run: `getComputedStyle(document.documentElement).getPropertyValue('--rd-space-6')` -- returns `24px`
- [ ] Run: `getComputedStyle(document.documentElement).getPropertyValue('--rd-duration-fast')` -- returns `120ms`
- [ ] Run: `getComputedStyle(document.documentElement).getPropertyValue('--rd-grain-opacity')` -- returns `0.035`

### Glass Panel Rendering

- [ ] A `.glass` element shows translucent background with content blurred behind it
- [ ] Confirm `backdrop-filter` is active in DevTools Computed tab (not grayed out)
- [ ] Hover a `.glassInteractive` element: panel lifts up-and-right, returns slowly
- [ ] Tab to a `.glass` element: accent-colored outline appears at 2px offset
- [ ] `.panelActive` shows 2px rose-dim left border

### Atmosphere Layers

- [ ] Grain overlay visible as subtle noise texture over all content
- [ ] Grain animation runs (shift pattern) -- verify in DevTools Animations panel
- [ ] Vignette darkens screen edges, center remains clear
- [ ] Scanlines visible at high zoom (200%+) as faint horizontal lines
- [ ] Rose wash invisible when `--rd-activity` is `0`
- [ ] Set `--rd-activity` to `0.15` via DevTools: faint rose glow appears at center
- [ ] All six atmosphere divs have `pointer-events: none` (click through to content below)

### Grain at 60fps

- [ ] Open DevTools Performance tab, record 5 seconds
- [ ] Grain animation does not cause dropped frames
- [ ] Grain does not trigger layout recalculation (transforms only)
- [ ] Total paint time per frame stays under 4ms

### Reduced Motion

- [ ] Enable `prefers-reduced-motion: reduce` in DevTools Rendering settings
- [ ] Grain animation stops (verify in Animations panel)
- [ ] Grain opacity decreases to `0.02` (still visible, less intense)
- [ ] Rose wash transitions become instant (0ms duration)
- [ ] Glass panel hover still works but consider adding reduced-motion rules for panel transitions

### Font Loading

- [ ] Hard-refresh the page (Cmd+Shift+R): text appears immediately in fallback font
- [ ] After fonts load (~200ms): text shifts to Berkeley Mono or JetBrains Mono (no invisible flash)
- [ ] Network tab shows font files loaded with `preload` priority
- [ ] Hero block number displays in Fraunces italic
- [ ] Address labels display in monospace at 10px with wide tracking

### Cross-Browser

- [ ] Chrome 120+: all features work (backdrop-filter, grain, scrollbar styling)
- [ ] Firefox 120+: backdrop-filter works, scrollbar uses `scrollbar-width: thin`
- [ ] Safari 17+: `-webkit-backdrop-filter` prefix active, grain renders
- [ ] Mobile Safari: no 3D (MOSAIC mode default), panels full-width, no hover effects
