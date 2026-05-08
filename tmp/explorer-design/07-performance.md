# Performance Strategy

How to keep a real-time 3D blockchain explorer running at 60fps without melting laptops.

---

## Performance Budgets

### Frame Budget

At 60fps, each frame has **16.6ms**. Budget allocation:

```
┌─────────────────────────────────────────────────────┐
│ 16.6ms frame budget                                  │
│                                                      │
│ ┌──────────────┐ ┌──────────┐ ┌────┐ ┌────┐ ┌────┐ │
│ │ Scene render  │ │ Particle │ │ UI │ │ GC │ │ ↕  │ │
│ │ (Three.js)   │ │ update   │ │    │ │    │ │buf │ │
│ │  6ms max     │ │  4ms max │ │2ms │ │1ms │ │3ms │ │
│ └──────────────┘ └──────────┘ └────┘ └────┘ └────┘ │
│  Scene (10ms)     React (2ms)  GC     Buffer        │
└─────────────────────────────────────────────────────┘
```

| Component | Budget | Notes |
|-----------|--------|-------|
| Three.js scene render | 6ms | Single draw call batching via InstancedMesh |
| WebGL2 particle update | 4ms | Transform feedback, GPU-side, minimal JS |
| React panel updates | 2ms | Only on state change, not every frame |
| GC pressure | 1ms | Keep allocations minimal in render loop |
| Buffer (headroom) | 3.6ms | Absorbs spikes, prevents frame drops |

### Network Budget

| Data Source | Frequency | Payload | Bandwidth |
|-------------|-----------|---------|-----------|
| Block poll (Phase 1) | 1/sec | ~2KB (empty block) | ~2 KB/s |
| Block poll (with txs) | 1/sec | ~10-50KB | ~50 KB/s peak |
| Fee history poll | 1/5sec | ~4KB | ~0.8 KB/s |
| WS newHeads (Phase 2) | 1/sec | ~1KB header | ~1 KB/s |
| WS pendingTxs (Phase 2) | variable | ~0.2KB each | ~20 KB/s peak |
| On-demand (user click) | rare | ~1-5KB | burst |
| **Total steady state** | | | **~5-75 KB/s** |

### Memory Budget

| Component | Allocation | Growth |
|-----------|-----------|--------|
| JS Heap (data layer) | 20MB | Bounded by LRU (500 blocks) |
| React component tree | 5MB | Bounded (fixed component count) |
| Three.js scene graph | 15MB | Bounded (ring buffer of 200 tiles) |
| WebGL2 particle buffers | 20MB | Fixed allocation (16K particles) |
| WebGL2 textures | 10MB | Bounded (no texture streaming) |
| Canvas2D (mosaic tiles) | 10MB | Bounded (visible tiles + small cache) |
| Audio (Tone.js) | 5MB | Fixed (loaded lazily, optional) |
| **Total target** | **< 150MB** | **Steady state, no growth** |

---

## Render Loop Architecture

### Single RAF Loop

One `requestAnimationFrame` drives everything. No competing loops.

```typescript
// apps/explorer/src/scenes/SceneManager.tsx

let lastTime = 0;

function tick(time: DOMHighResTimeStamp) {
  const dt = Math.min((time - lastTime) / 1000, 0.05); // cap at 50ms (20fps min)
  lastTime = time;

  // 1. Update active scene (Three.js or regl)
  activeScene?.update(dt);

  // 2. Render active scene
  activeScene?.render();

  // 3. UI updates happen independently via React (not in this loop)

  requestAnimationFrame(tick);
}

requestAnimationFrame(tick);
```

### Frame Budget Monitoring

```typescript
// Development-only performance overlay
class FrameMonitor {
  private frameTimes: number[] = [];
  private readonly WINDOW = 120; // 2 seconds at 60fps

  recordFrame(startTime: number) {
    const duration = performance.now() - startTime;
    this.frameTimes.push(duration);
    if (this.frameTimes.length > this.WINDOW) this.frameTimes.shift();

    // Warn if consistently over budget
    if (this.frameTimes.length === this.WINDOW) {
      const avg = this.frameTimes.reduce((a, b) => a + b) / this.WINDOW;
      const p95 = this.frameTimes.sort()[Math.floor(this.WINDOW * 0.95)];
      if (p95 > 16.6) {
        console.warn(`Frame budget exceeded: avg=${avg.toFixed(1)}ms p95=${p95.toFixed(1)}ms`);
      }
    }
  }
}
```

---

## Scene Optimization Strategies

### Hash Terrain

**Draw call batching**: All terrain tiles share one `BufferGeometry` and one `ShaderMaterial`. Tile data is packed into a single vertex buffer, updated per-block for the new tile only (not the entire ring).

```
Tiles: 200 max → 200 × 256 vertices = 51,200 vertices
Draw calls: 1 (single merged geometry)
Triangles: ~100,000 (acceptable for any modern GPU)
```

**Tile recycling**: When a new block arrives, the oldest tile's vertex data is overwritten in-place. No allocation, no GC.

**Frustum culling**: Only tiles within the camera frustum are submitted to the GPU. With a 15° FOV, typically 30-40 tiles visible. The other 160+ are skipped entirely.

**LOD**: Tiles beyond z=30 from camera could be simplified (8×8 grid instead of 16×16), but at 100K triangles total, this is premature — skip until profiling demands it.

### Particle Constellation

**Transform feedback**: Particle physics runs entirely on GPU. The JS side only:
1. Spawns new particles (writes to buffer, ~20 per transaction)
2. Sets uniforms (time, delta)
3. Issues draw call

No per-particle JS iteration. No JS → GPU data copy per frame (except new spawns).

**Point sprites**: Particles rendered as `GL_POINTS` with `gl_PointSize`. One vertex per particle, fragment shader draws soft circle. Far cheaper than quads.

**Pool allocation**: Particle buffer is pre-allocated for 16,384 particles. No runtime allocation. Dead particles (lifetime > 1.0) are overwritten by new spawns via a free-list.

### Block Waterfall

**Canvas2D, not WebGL**: The waterfall is 50 rectangles with simple fills. Canvas2D is more appropriate than WebGL — less setup, no shader compilation, CSS-animatable.

**Offscreen rendering**: Each block's hash barcode pattern is rendered once to an offscreen canvas, then cached as an `ImageBitmap`. Subsequent frames just `drawImage()`.

**CSS for falling animation**: Block entry animation uses CSS `transform` + `transition` (compositor-only). The canvas redraws only when a new block arrives, not every frame.

---

## React Optimization

### Avoiding Unnecessary Re-renders

```typescript
// WRONG: entire component re-renders on any store change
const { latestBlock, blocks, addresses } = useChainStore();

// RIGHT: subscribe to specific slices
const latestBlockNumber = useChainStore((s) => s.latestBlockNumber);
const connectionStatus = useChainStore((s) => s.status);
```

Zustand's selector pattern ensures components only re-render when their specific data changes. A new block arriving doesn't re-render the search panel.

### Scene-React Boundary

Scenes (Three.js, regl) run outside React's render cycle. They subscribe to the EventBus directly:

```typescript
// Inside a scene's useEffect:
useEffect(() => {
  const unsub = bus.on('block:new', (block) => {
    // Imperatively update scene data — NO setState
    terrainRing.addBlock(block);
  });
  return unsub;
}, []);
```

This means: **zero React re-renders per block for scene updates**. React only re-renders for panel data (block number display, fee chart, status LED).

### Lazy Loading

```typescript
// Scene chunks loaded on first use
const TerrainScene = React.lazy(() => import('./scenes/terrain/TerrainScene'));
const ConstellationCanvas = React.lazy(() => import('./scenes/constellation/ConstellationCanvas'));
const WaterfallScene = React.lazy(() => import('./scenes/waterfall/WaterfallScene'));

// Audio loaded only if user enables it
const AudioEngine = React.lazy(() => import('./audio/AudioEngine'));
```

Initial page load includes only: React + Zustand + router + panels + data layer (~80KB gz). Three.js (~120KB gz) loads when the user first sees Terrain mode.

---

## Data Layer Optimization

### Polling Efficiency

```typescript
// Avoid redundant fetches:
// 1. Only fetch blocks we don't already have
// 2. Only fetch full block if it has transactions
async pollBlock() {
  const num = await this.rpc("eth_blockNumber");
  if (num <= this.lastBlock) return;

  for (let n = this.lastBlock + 1n; n <= num; n++) {
    // First: get block WITHOUT transaction bodies
    const block = await this.rpc("eth_getBlockByNumber", [hex(n), false]);

    if (block.transactions.length > 0) {
      // Only fetch full bodies if there are transactions
      const fullBlock = await this.rpc("eth_getBlockByNumber", [hex(n), true]);
      this.bus.emit("block:new", fullBlock);
    } else {
      this.bus.emit("block:new", block);
    }
  }
}
```

This halves RPC bandwidth on mostly-empty chains (like kora today).

### Subscription Deduplication (Phase 2)

When WebSocket subscriptions are active:
- `newHeads` replaces block number polling
- Block detail fetch still needed (headers don't include tx bodies)
- Fee history still polled (no subscription equivalent)
- Pending tx subscription feeds directly to constellation scene

If WS disconnects, fall back to polling seamlessly.

---

## Progressive Enhancement Tiers

| Tier | Hardware | Features | FPS Target |
|------|----------|----------|------------|
| **Full** | Desktop, dGPU, 8GB+ RAM | All scenes, particles, audio, atmosphere | 60 |
| **Standard** | Desktop/laptop, iGPU, 4GB+ RAM | Terrain + waterfall, reduced particles (8K), atmosphere | 60 |
| **Light** | Low-end laptop, 2GB RAM | Mosaic only (Canvas2D), no 3D, no particles | 30 |
| **Mobile** | Phone/tablet | Mosaic only, simplified panels, no atmosphere | 30 |

### Detection

```typescript
function detectTier(): 'full' | 'standard' | 'light' | 'mobile' {
  // Mobile detection
  if (/Mobi|Android/i.test(navigator.userAgent)) return 'mobile';
  if (window.innerWidth < 760) return 'mobile';

  // GPU capability (WebGL2 required for 'standard' and above)
  const canvas = document.createElement('canvas');
  const gl = canvas.getContext('webgl2');
  if (!gl) return 'light';

  // RAM detection
  const memory = (navigator as any).deviceMemory ?? 8; // default assume 8GB
  if (memory < 4) return 'light';

  // GPU renderer string heuristic
  const debugInfo = gl.getExtension('WEBGL_debug_renderer_info');
  if (debugInfo) {
    const renderer = gl.getParameter(debugInfo.UNMASKED_RENDERER_WEBGL);
    // Integrated GPUs: Intel HD/UHD, Apple M-series (still fast), AMD APUs
    const isIntegrated = /Intel|Mali|Adreno/i.test(renderer)
      && !/Arc/i.test(renderer); // Intel Arc is discrete
    if (isIntegrated && memory < 8) return 'standard';
  }

  return 'full';
}
```

### Per-Tier Configuration

```typescript
const TIER_CONFIG = {
  full: {
    terrainRingSize: 200,
    maxParticles: 16384,
    blockCacheSize: 500,
    atmosphereLayers: true,
    audioAvailable: true,
    scenesEnabled: ['terrain', 'constellation', 'waterfall', 'consensus', 'aurora', 'organism'],
  },
  standard: {
    terrainRingSize: 100,
    maxParticles: 8192,
    blockCacheSize: 300,
    atmosphereLayers: true,
    audioAvailable: true,
    scenesEnabled: ['terrain', 'waterfall'],
  },
  light: {
    terrainRingSize: 0,
    maxParticles: 0,
    blockCacheSize: 200,
    atmosphereLayers: false,
    audioAvailable: false,
    scenesEnabled: [],
  },
  mobile: {
    terrainRingSize: 0,
    maxParticles: 0,
    blockCacheSize: 100,
    atmosphereLayers: false,
    audioAvailable: false,
    scenesEnabled: [],
  },
};
```

---

## Monitoring & Debugging

### Development Overlay

Press `Ctrl+Shift+P` in development to toggle the performance overlay:

```
┌─ PERF ─────────────────────────────┐
│  FPS    60 ▁▁▁▁▁▁▁▁▁▁▁▁▁▁▁▁▁▁▁▁  │
│  FRAME  4.2ms (p95: 8.1ms)         │
│  HEAP   42MB / 150MB               │
│  GPU    TerrainMesh: 1 draw call   │
│         Particles: 4,201 active    │
│  NET    2.1 KB/s ↓  0.1 KB/s ↑    │
│  CACHE  blocks: 127  addrs: 341   │
│  RPC    polls: 142  errors: 0     │
└─────────────────────────────────────┘
```

### Production Error Reporting

Uncaught errors in scenes should NOT crash the entire app:

```typescript
// ErrorBoundary around each scene
<ErrorBoundary fallback={<SceneError />}>
  <Suspense fallback={<SceneLoading />}>
    <TerrainScene />
  </Suspense>
</ErrorBoundary>
```

If a WebGL context is lost (GPU driver crash, tab backgrounded too long):
1. Detect via `canvas.addEventListener('webglcontextlost')`
2. Show "Reconnecting to GPU..." message
3. Attempt context restoration via `webglcontextrestored` event
4. If restoration fails, fall back to MOSAIC mode (Canvas2D)
