# 05 -- Scene: Block Terrain

Block hashes rendered as heightmap terrain tiles, scrolling infinitely along the Z axis.
The signature visual of the Kora Explorer.

---

## Architecture

```
                     EventBus
                        |
                  "block:new"
                        |
                        v
              +-------------------+
              |   TerrainScene    |   R3F <Canvas>
              |   (container)     |   fog, camera, lights
              +--------+----------+
                       |
          +------------+------------+
          |                         |
  +-------v--------+     +---------v---------+
  |  TerrainRing   |     |  TransactionBeams |
  |  (tile mgr)    |     |  (per-block)      |
  +-------+--------+     +-------------------+
          |
  +-------v--------+
  |  TerrainTile   |   x10 slots (8 visible + 2 pre-gen)
  |  (mesh)        |   PlaneGeometry + ShaderMaterial
  +-------+--------+
          |
  +-------v--------+
  | hashToHeightmap |   block.hash -> Float32Array[1024]
  +----------------+
```

- **React Three Fiber** (R3F) Canvas component with `@react-three/fiber` v9
- Custom terrain geometry generated deterministically from each block hash
- Ring buffer of 10 terrain tiles (8 visible within fog, 2 pre-generated ahead)
- Each tile = 1 block, positioned along the negative Z axis (receding into fog)
- Custom `ShaderMaterial` with vertex displacement and height-based coloring
- Transaction markers as additive-blended light beams per block

### File Manifest

```
apps/explorer/src/scenes/terrain/
  TerrainScene.tsx        -- 5.1  R3F Canvas container
  hashmap.ts              -- 5.2  hash-to-heightmap algorithm
  TerrainTile.tsx          -- 5.3  individual tile mesh
  terrain.vert.glsl       -- 5.3  vertex shader
  terrain.frag.glsl       -- 5.3  fragment shader
  TerrainRing.ts          -- 5.4  ring buffer manager
  TransactionBeams.tsx    -- 5.3  tx marker beams
  interactions.ts         -- 5.5  raycasting, selection
  index.ts                -- barrel export
```

---

## 5.1 Terrain Scene Container

The top-level R3F Canvas that owns camera, lighting, fog, and child components.

- [ ] `TerrainScene.tsx` -- R3F `<Canvas>` with fog, camera, lights
- [ ] PerspectiveCamera at `[0, 8, 12]`, lookAt `[0, 0, -4]`, FOV 15 degrees
- [ ] Exponential fog: color matches `--rd-void` (#060608), near=10, far=60
- [ ] Ambient light intensity 0.15, rose-tinted `#3a2030`
- [ ] Directional light from upper-left `[-4, 8, 4]`, intensity 0.7, no shadow
- [ ] Mouse tilt: pointer position gently tilts camera +/-5 degrees on both axes, lerp factor 0.04
- [ ] Subscribe to `bus.on('block:new')` imperatively (not via Zustand) to avoid re-renders
- [ ] Wrap in `<ErrorBoundary>` + `<Suspense>` per performance spec

```typescript
// apps/explorer/src/scenes/terrain/TerrainScene.tsx

import { Canvas, useFrame, useThree } from '@react-three/fiber';
import { PerspectiveCamera } from '@react-three/drei';
import { useRef, useEffect, useMemo, Suspense } from 'react';
import * as THREE from 'three';
import { bus } from '@/data/bus';
import { TerrainRing } from './TerrainRing';
import { TerrainTileMesh } from './TerrainTile';
import { TransactionBeams } from './TransactionBeams';
import type { ChainBlock } from '@/data/types';

const FOG_COLOR = new THREE.Color(0x060608);
const AMBIENT_COLOR = new THREE.Color(0x3a2030);

/** Manages the camera tilt driven by pointer position. */
function CameraController() {
  const { camera } = useThree();
  const targetRotX = useRef(0);
  const targetRotY = useRef(0);
  const currentRotX = useRef(0);
  const currentRotY = useRef(0);

  useEffect(() => {
    const onPointerMove = (e: PointerEvent) => {
      // Normalize pointer to [-1, 1]
      const nx = (e.clientX / window.innerWidth) * 2 - 1;
      const ny = (e.clientY / window.innerHeight) * 2 - 1;
      // +/- 5 degrees in radians
      const MAX_TILT = (5 * Math.PI) / 180;
      targetRotX.current = -ny * MAX_TILT;
      targetRotY.current = nx * MAX_TILT;
    };
    window.addEventListener('pointermove', onPointerMove, { passive: true });
    return () => window.removeEventListener('pointermove', onPointerMove);
  }, []);

  useFrame(() => {
    // Glacial lerp -- 0.04 per frame at 60fps
    const LERP = 0.04;
    currentRotX.current += (targetRotX.current - currentRotX.current) * LERP;
    currentRotY.current += (targetRotY.current - currentRotY.current) * LERP;
    camera.rotation.x = currentRotX.current + camera.userData.baseRotX;
    camera.rotation.y = currentRotY.current;
  });

  return null;
}

/** Inner scene content rendered within the R3F Canvas. */
function TerrainSceneContent() {
  const ring = useMemo(() => new TerrainRing(10), []);
  const groupRef = useRef<THREE.Group>(null!);

  // Subscribe to new blocks imperatively -- no React re-renders
  useEffect(() => {
    const unsub = bus.on('block:new', (block: ChainBlock) => {
      ring.advance(block);
    });
    return unsub;
  }, [ring]);

  // Animate the scroll: tiles move toward camera at a constant speed
  useFrame((_, delta) => {
    ring.update(delta);
  });

  return (
    <>
      {/* Camera */}
      <PerspectiveCamera
        makeDefault
        position={[0, 8, 12]}
        fov={15}
        near={0.1}
        far={100}
        onUpdate={(cam) => {
          cam.lookAt(0, 0, -4);
          cam.userData.baseRotX = cam.rotation.x;
        }}
      />

      {/* Fog */}
      <fogExp2 attach="fog" args={[FOG_COLOR, 0.025]} />

      {/* Lighting */}
      <ambientLight color={AMBIENT_COLOR} intensity={0.15} />
      <directionalLight
        position={[-4, 8, 4]}
        intensity={0.7}
        color={0xffeedd}
      />

      {/* Camera tilt controller */}
      <CameraController />

      {/* Terrain tiles */}
      <group ref={groupRef}>
        {ring.getSlots().map((slot) => (
          <TerrainTileMesh key={slot.id} slot={slot} />
        ))}
      </group>

      {/* Transaction beams overlay */}
      <TransactionBeams ring={ring} />
    </>
  );
}

/** Top-level exported component. Lazy-loaded by SceneManager. */
export default function TerrainScene() {
  return (
    <Canvas
      gl={{
        antialias: true,
        alpha: true,
        powerPreference: 'high-performance',
      }}
      style={{ position: 'absolute', inset: 0 }}
      dpr={[1, 2]}
    >
      <Suspense fallback={null}>
        <TerrainSceneContent />
      </Suspense>
    </Canvas>
  );
}
```

---

## 5.2 Hash-to-Heightmap Algorithm

Deterministically converts a 32-byte block hash into a 32x32 heightmap grid.
Five noise layers with increasing frequency, plus an erosion pass.

- [ ] Implement `hashToHeightmap(hash, gridSize)` returning `Float32Array`
- [ ] Layer 0: bytes 0-7 -- large-scale ridges (amplitude 1.0, freq 0.02)
- [ ] Layer 1: bytes 8-15 -- medium features (amplitude 0.5, freq 0.04)
- [ ] Layer 2: bytes 16-23 -- small detail (amplitude 0.25, freq 0.08)
- [ ] Layer 3: bytes 24-27 -- micro texture (amplitude 0.125, freq 0.16)
- [ ] Layer 4: erosion pass (3 iterations, thermal + hydraulic)
- [ ] Returns `gridSize * gridSize` Float32Array of heights in `[0, 1]`
- [ ] Implement seeded PRNG (`mulberry32`) from hash bytes
- [ ] Implement 2D Perlin noise function (`perlin2D`) seeded by hash
- [ ] Implement color parameters extraction (hue shift, saturation) from hash bytes 16-24
- [ ] Test: same hash always produces identical heightmap
- [ ] Test: visually distinct hashes produce visibly different terrain

```typescript
// apps/explorer/src/scenes/terrain/hashmap.ts

// ─── Utility ────────────────────────────────────────────────

/** Decode a hex hash string into a Uint8Array of 32 bytes. */
function hexToBytes(hex: `0x${string}`): Uint8Array {
  const clean = hex.startsWith('0x') ? hex.slice(2) : hex;
  const bytes = new Uint8Array(32);
  for (let i = 0; i < 32; i++) {
    bytes[i] = parseInt(clean.slice(i * 2, i * 2 + 2), 16);
  }
  return bytes;
}

/** Read 4 bytes at `offset` as a big-endian uint32. */
function readUint32(bytes: Uint8Array, offset: number): number {
  return (
    ((bytes[offset] << 24) |
      (bytes[offset + 1] << 16) |
      (bytes[offset + 2] << 8) |
      bytes[offset + 3]) >>>
    0
  );
}

/** Normalize a byte range [start, end) to a float in [0, 1]. */
function bytesToNorm(bytes: Uint8Array, start: number, end: number): number {
  let val = 0;
  for (let i = start; i < end; i++) val = (val * 256 + bytes[i]) >>> 0;
  return val / 0xffffffff;
}

function clamp(v: number, lo: number, hi: number): number {
  return v < lo ? lo : v > hi ? hi : v;
}

function lerp(a: number, b: number, t: number): number {
  return a + (b - a) * t;
}

// ─── Seeded PRNG ────────────────────────────────────────────

/** Mulberry32: fast 32-bit seeded PRNG. Returns a function that yields [0, 1). */
function mulberry32(seed: number): () => number {
  let s = seed | 0;
  return () => {
    s = (s + 0x6d2b79f5) | 0;
    let t = Math.imul(s ^ (s >>> 15), 1 | s);
    t = (t + Math.imul(t ^ (t >>> 7), 61 | t)) ^ t;
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
}

// ─── Perlin Noise (2D, seeded) ──────────────────────────────

/** Permutation table seeded by PRNG. 512 entries (doubled for wrap). */
function buildPermTable(rng: () => number): Uint8Array {
  const perm = new Uint8Array(512);
  // Fisher-Yates shuffle of 0..255
  for (let i = 0; i < 256; i++) perm[i] = i;
  for (let i = 255; i > 0; i--) {
    const j = (rng() * (i + 1)) | 0;
    const tmp = perm[i];
    perm[i] = perm[j];
    perm[j] = tmp;
  }
  // Double for wrap-around
  for (let i = 0; i < 256; i++) perm[i + 256] = perm[i];
  return perm;
}

const GRAD2 = [
  [1, 1], [-1, 1], [1, -1], [-1, -1],
  [1, 0], [-1, 0], [0, 1], [0, -1],
];

function fade(t: number): number {
  return t * t * t * (t * (t * 6 - 15) + 10);
}

function grad2d(hash: number, x: number, y: number): number {
  const g = GRAD2[hash & 7];
  return g[0] * x + g[1] * y;
}

/** Evaluate 2D Perlin noise at (x, y) using a seeded permutation table. */
function perlin2D(x: number, y: number, perm: Uint8Array): number {
  const xi = Math.floor(x) & 255;
  const yi = Math.floor(y) & 255;
  const xf = x - Math.floor(x);
  const yf = y - Math.floor(y);

  const u = fade(xf);
  const v = fade(yf);

  const aa = perm[perm[xi] + yi];
  const ab = perm[perm[xi] + yi + 1];
  const ba = perm[perm[xi + 1] + yi];
  const bb = perm[perm[xi + 1] + yi + 1];

  const x1 = lerp(grad2d(aa, xf, yf), grad2d(ba, xf - 1, yf), u);
  const x2 = lerp(grad2d(ab, xf, yf - 1), grad2d(bb, xf - 1, yf - 1), u);

  return lerp(x1, x2, v);
}

// ─── Erosion Pass ───────────────────────────────────────────

/**
 * Thermal erosion: material slides from steep cells to lower neighbors.
 * Modifies `heights` in place.
 */
function thermalErosion(
  heights: Float32Array,
  size: number,
  talusAngle: number,
): void {
  const transfer = 0.3; // fraction of excess to move per iteration
  for (let y = 1; y < size - 1; y++) {
    for (let x = 1; x < size - 1; x++) {
      const i = y * size + x;
      const h = heights[i];

      // 4-connected neighbors
      const neighbors = [
        (y - 1) * size + x,
        (y + 1) * size + x,
        y * size + (x - 1),
        y * size + (x + 1),
      ];

      let maxDiff = 0;
      let lowestIdx = -1;

      for (const ni of neighbors) {
        const diff = h - heights[ni];
        if (diff > talusAngle && diff > maxDiff) {
          maxDiff = diff;
          lowestIdx = ni;
        }
      }

      if (lowestIdx >= 0) {
        const amount = (maxDiff - talusAngle) * transfer;
        heights[i] -= amount;
        heights[lowestIdx] += amount;
      }
    }
  }
}

/**
 * Hydraulic erosion: simulates water flow carrying sediment downhill.
 * A simplified droplet model applied per cell.
 */
function hydraulicErosion(
  heights: Float32Array,
  size: number,
  rng: () => number,
): void {
  const droplets = size * 2; // number of simulated rain droplets
  for (let d = 0; d < droplets; d++) {
    let x = (rng() * (size - 2) + 1) | 0;
    let y = (rng() * (size - 2) + 1) | 0;
    let sediment = 0;
    const erosionRate = 0.01;
    const depositionRate = 0.005;

    for (let step = 0; step < 16; step++) {
      const i = y * size + x;
      const h = heights[i];

      // Find steepest downhill neighbor
      const neighbors = [
        { nx: x, ny: y - 1 },
        { nx: x, ny: y + 1 },
        { nx: x - 1, ny: y },
        { nx: x + 1, ny: y },
      ];

      let best = { nx: x, ny: y };
      let bestDiff = 0;

      for (const n of neighbors) {
        if (n.nx < 0 || n.nx >= size || n.ny < 0 || n.ny >= size) continue;
        const diff = h - heights[n.ny * size + n.nx];
        if (diff > bestDiff) {
          bestDiff = diff;
          best = n;
        }
      }

      if (bestDiff <= 0) {
        // Deposit sediment in flat area
        heights[i] += sediment * depositionRate;
        break;
      }

      // Erode current cell, deposit some sediment
      heights[i] -= erosionRate;
      sediment += erosionRate;
      if (sediment > 0.05) {
        heights[i] += sediment * depositionRate;
        sediment *= 1 - depositionRate;
      }

      x = best.nx;
      y = best.ny;
    }
  }
}

// ─── Main Heightmap Generator ───────────────────────────────

/**
 * Terrain parameters extracted from hash bytes.
 * Exposed so the shader can read hueShift / saturation.
 */
export interface TerrainParams {
  baseElevation: number; // 0-1 from bytes[0..4]
  roughness: number;     // 0-1 from bytes[4..8]
  ridgeFactor: number;   // 0-1 from bytes[8..12]
  erosion: number;       // 0-1 from bytes[12..16]
  hueShift: number;      // 0-1 from bytes[16..20]
  saturation: number;    // 0-1 from bytes[20..24]
  blendCurve: number;    // 0-1 from bytes[24..28]
  features: number;      // 0-1 from bytes[28..32]
}

/** Extract terrain parameters from a 32-byte hash. */
export function extractParams(hash: `0x${string}`): TerrainParams {
  const bytes = hexToBytes(hash);
  return {
    baseElevation: bytesToNorm(bytes, 0, 4),
    roughness: bytesToNorm(bytes, 4, 8),
    ridgeFactor: bytesToNorm(bytes, 8, 12),
    erosion: bytesToNorm(bytes, 12, 16),
    hueShift: bytesToNorm(bytes, 16, 20),
    saturation: bytesToNorm(bytes, 20, 24),
    blendCurve: bytesToNorm(bytes, 24, 28),
    features: bytesToNorm(bytes, 28, 32),
  };
}

/**
 * Convert a block hash into a heightmap.
 *
 * Algorithm:
 *   5 noise layers at increasing frequency, seeded from hash byte groups.
 *   Followed by an erosion pass (thermal + hydraulic, 3 iterations).
 *
 * @param hash    - 0x-prefixed 32-byte hex block hash
 * @param gridSize - vertex resolution per axis (default 32)
 * @returns Float32Array of gridSize*gridSize heights in [0, 1]
 */
export function hashToHeightmap(
  hash: `0x${string}`,
  gridSize: number = 32,
): Float32Array {
  const bytes = hexToBytes(hash);
  const params = extractParams(hash);
  const total = gridSize * gridSize;
  const heights = new Float32Array(total);

  // Seed PRNG from first 4 bytes
  const rng = mulberry32(readUint32(bytes, 0));
  const perm = buildPermTable(rng);

  // Layer configurations derived from hash byte groups:
  //
  //   Layer 0 (bytes 0-7):   large-scale ridges
  //   Layer 1 (bytes 8-15):  medium features
  //   Layer 2 (bytes 16-23): small detail
  //   Layer 3 (bytes 24-27): micro texture
  //
  // Each layer's seed offset comes from its byte group to decorrelate
  // the noise across layers.
  const layers = [
    { amp: 1.0,   freq: 0.02, seedOffset: readUint32(bytes, 0) },
    { amp: 0.5,   freq: 0.04, seedOffset: readUint32(bytes, 8) },
    { amp: 0.25,  freq: 0.08, seedOffset: readUint32(bytes, 16) },
    { amp: 0.125, freq: 0.16, seedOffset: readUint32(bytes, 24) },
  ];

  for (let y = 0; y < gridSize; y++) {
    for (let x = 0; x < gridSize; x++) {
      const i = y * gridSize + x;
      const nx = x / (gridSize - 1); // normalized [0, 1]
      const ny = y / (gridSize - 1);

      let h = 0;
      let ampSum = 0;

      // ── Noise layers ──
      for (const layer of layers) {
        const sx = (nx + layer.seedOffset * 0.001) * gridSize * layer.freq;
        const sy = (ny + layer.seedOffset * 0.0007) * gridSize * layer.freq;
        h += perlin2D(sx, sy, perm) * layer.amp;
        ampSum += layer.amp;
      }

      // Normalize to [0, 1]
      h = (h / ampSum) * 0.5 + 0.5;

      // ── Ridge layer ──
      // Absolute-value noise creates sharp ridges when mixed in.
      const ridgeNx = nx * gridSize * 0.03 + readUint32(bytes, 8) * 0.001;
      const ridgeNy = ny * gridSize * 0.03 + readUint32(bytes, 12) * 0.001;
      const ridge = 1.0 - Math.abs(perlin2D(ridgeNx, ridgeNy, perm));
      h = lerp(h, ridge, params.ridgeFactor * 0.6);

      // ── Base elevation offset ──
      h = h * 0.7 + params.baseElevation * 0.3;

      // ── Feature placement ──
      // A single prominent peak or valley derived from the feature parameter.
      const featureX = params.features * (gridSize - 1);
      const featureY = (1 - params.features) * (gridSize - 1);
      const distToFeature = Math.hypot(x - featureX, y - featureY) / gridSize;
      h += Math.max(0, 0.3 - distToFeature) * params.features;

      heights[i] = clamp(h, 0, 1);
    }
  }

  // ── Erosion pass ──
  // Only applied when the erosion parameter exceeds the threshold.
  // 3 iterations: 2 thermal + 1 hydraulic.
  if (params.erosion > 0.2) {
    const talusAngle = 0.05 + (1 - params.erosion) * 0.1;
    for (let iter = 0; iter < 2; iter++) {
      thermalErosion(heights, gridSize, talusAngle);
    }
    hydraulicErosion(heights, gridSize, rng);
  }

  // ── Normalize to [0, 1] post-erosion ──
  let min = Infinity;
  let max = -Infinity;
  for (let i = 0; i < total; i++) {
    if (heights[i] < min) min = heights[i];
    if (heights[i] > max) max = heights[i];
  }
  const range = max - min || 1;
  for (let i = 0; i < total; i++) {
    heights[i] = (heights[i] - min) / range;
  }

  return heights;
}

// ─── Color mapping ──────────────────────────────────────────

/** ROSEDUST terrain palette -- maps height to RGB. */
export const TERRAIN_PALETTE = {
  deep:   { r: 0x0a / 255, g: 0x0f / 255, b: 0x1a / 255 }, // #0a0f1a -- valleys
  mid:    { r: 0x3a / 255, g: 0x20 / 255, b: 0x30 / 255 }, // #3a2030 -- rose-deep
  high:   { r: 0xdc / 255, g: 0xa5 / 255, b: 0xbd / 255 }, // #dca5bd -- rose-glow
  peak:   { r: 0xd8 / 255, g: 0xc8 / 255, b: 0xa0 / 255 }, // #d8c8a0 -- bone-bright
} as const;

/**
 * Map a height value [0, 1] to an RGB color triple from the terrain palette.
 * Used for CPU-side color computation (e.g., minimap, thumbnails).
 * The shader does this on the GPU (see terrain.frag.glsl).
 */
export function heightToColor(
  height: number,
  hueShift: number,
): [number, number, number] {
  const { deep, mid, high, peak } = TERRAIN_PALETTE;
  let r: number, g: number, b: number;

  if (height < 0.33) {
    const t = height / 0.33;
    r = lerp(deep.r, mid.r, t);
    g = lerp(deep.g, mid.g, t);
    b = lerp(deep.b, mid.b, t);
  } else if (height < 0.66) {
    const t = (height - 0.33) / 0.33;
    r = lerp(mid.r, high.r, t);
    g = lerp(mid.g, high.g, t);
    b = lerp(mid.b, high.b, t);
  } else {
    const t = (height - 0.66) / 0.34;
    r = lerp(high.r, peak.r, t);
    g = lerp(high.g, peak.g, t);
    b = lerp(high.b, peak.b, t);
  }

  // Apply hue rotation (subtle, max +/- 15% of a full turn)
  const angle = (hueShift - 0.5) * 0.3 * Math.PI * 2;
  const cos = Math.cos(angle);
  const sin = Math.sin(angle);
  const rr = r * (0.299 + 0.701 * cos) + g * (0.587 - 0.587 * cos) + b * (0.114 - 0.114 * cos);
  const gg = r * (0.299 - 0.299 * cos + 0.328 * sin) + g * (0.587 + 0.413 * cos) + b * (0.114 - 0.114 * cos - 0.328 * sin);
  const bb = r * (0.299 - 0.299 * cos - 1.25 * sin) + g * (0.587 - 0.587 * cos + 1.05 * sin) + b * (0.114 + 0.886 * cos + 0.203 * sin);

  return [clamp(rr, 0, 1), clamp(gg, 0, 1), clamp(bb, 0, 1)];
}
```

---

## 5.3 Terrain Tile Mesh

Each tile is a `PlaneGeometry(8, 8, 31, 31)` with vertex Y displacement driven by the heightmap, rendered with a custom `ShaderMaterial`.

- [ ] `TerrainTile.tsx` -- R3F mesh from heightmap data
- [ ] PlaneGeometry `(8, 8, 31, 31)` giving 32x32 vertices matching the heightmap grid
- [ ] Custom `ShaderMaterial` with vertex + fragment shaders
- [ ] Heightmap data passed as a `DataTexture` (32x32 single-channel float)
- [ ] Vertex shader: sample heightmap texture, displace vertex Y
- [ ] Fragment shader: height-based color gradient, fog, hue rotation from hash
- [ ] Transaction markers: small additive-blended billboard quads at positions from tx hashes
- [ ] Activity level uniform: empty blocks have max height 0.6, full blocks up to 2.0

```typescript
// apps/explorer/src/scenes/terrain/TerrainTile.tsx

import { useRef, useMemo, useEffect } from 'react';
import { useFrame } from '@react-three/fiber';
import * as THREE from 'three';
import vertexShader from './terrain.vert.glsl';
import fragmentShader from './terrain.frag.glsl';
import type { TerrainSlot } from './TerrainRing';

const TILE_SIZE = 8;        // world units per tile (X and Z)
const GRID_RES = 31;        // segments per axis (32 vertices)
const GRID_VERTS = 32;      // vertices per axis

interface Props {
  slot: TerrainSlot;
}

/**
 * Create a DataTexture from the heightmap Float32Array.
 * Single-channel RED float texture, 32x32.
 */
function createHeightTexture(heightmap: Float32Array): THREE.DataTexture {
  const tex = new THREE.DataTexture(
    heightmap,
    GRID_VERTS,
    GRID_VERTS,
    THREE.RedFormat,
    THREE.FloatType,
  );
  tex.minFilter = THREE.LinearFilter;
  tex.magFilter = THREE.LinearFilter;
  tex.wrapS = THREE.ClampToEdgeWrapping;
  tex.wrapT = THREE.ClampToEdgeWrapping;
  tex.needsUpdate = true;
  return tex;
}

export function TerrainTileMesh({ slot }: Props) {
  const meshRef = useRef<THREE.Mesh>(null!);

  // Geometry is shared across tiles (created once)
  const geometry = useMemo(() => {
    const geo = new THREE.PlaneGeometry(TILE_SIZE, TILE_SIZE, GRID_RES, GRID_RES);
    // Rotate plane to be horizontal (XZ plane, Y is up)
    geo.rotateX(-Math.PI / 2);
    return geo;
  }, []);

  // Material with custom shaders
  const material = useMemo(() => {
    return new THREE.ShaderMaterial({
      vertexShader,
      fragmentShader,
      uniforms: {
        uHeightmap: { value: null },
        uActivityLevel: { value: 0 },
        uHueShift: { value: 0.5 },
        uSaturation: { value: 0.5 },
        uTime: { value: 0 },
        uOpacity: { value: 1.0 },

        // Color palette (ROSEDUST)
        uColorDeep: { value: new THREE.Color(0x0a0f1a) },
        uColorMid: { value: new THREE.Color(0x3a2030) },
        uColorHigh: { value: new THREE.Color(0xdca5bd) },
        uColorPeak: { value: new THREE.Color(0xd8c8a0) },

        // Fog
        uFogColor: { value: new THREE.Color(0x060608) },
        uFogDensity: { value: 0.025 },
      },
      transparent: true,
      side: THREE.DoubleSide,
    });
  }, []);

  // Update material uniforms when slot data changes
  useEffect(() => {
    if (!slot.heightmap) return;

    const tex = createHeightTexture(slot.heightmap);
    material.uniforms.uHeightmap.value = tex;
    material.uniforms.uActivityLevel.value = slot.activityLevel;
    material.uniforms.uHueShift.value = slot.params.hueShift;
    material.uniforms.uSaturation.value = slot.params.saturation;
    material.uniforms.uOpacity.value = slot.opacity;

    return () => tex.dispose();
  }, [slot.heightmap, slot.activityLevel, slot.params, slot.opacity, material]);

  // Position tile along Z axis
  useFrame((_, delta) => {
    if (meshRef.current) {
      meshRef.current.position.z = slot.positionZ;
      material.uniforms.uTime.value += delta;
      material.uniforms.uOpacity.value = slot.opacity;
    }
  });

  return <mesh ref={meshRef} geometry={geometry} material={material} />;
}
```

### Vertex Shader

```glsl
// apps/explorer/src/scenes/terrain/terrain.vert.glsl
precision highp float;

uniform sampler2D uHeightmap;
uniform float uActivityLevel;
uniform float uTime;

varying float vHeight;
varying float vFogDepth;
varying vec2 vUv;

void main() {
  vUv = uv;

  // Sample heightmap texture at this vertex's UV
  float h = texture2D(uHeightmap, uv).r;
  vHeight = h;

  // Displace Y by height.
  // Activity level scales the amplitude:
  //   empty blocks (activity=0) -> max displacement 0.6
  //   full blocks  (activity=1) -> max displacement 2.0
  float maxHeight = 0.6 + uActivityLevel * 1.4;
  vec3 pos = position;
  pos.y += h * maxHeight;

  // Subtle wave animation on peak vertices (visual life)
  pos.y += sin(uTime * 0.5 + pos.x * 0.3 + pos.z * 0.2) * 0.02 * h;

  vec4 mvPos = modelViewMatrix * vec4(pos, 1.0);
  vFogDepth = -mvPos.z;

  gl_Position = projectionMatrix * mvPos;
}
```

### Fragment Shader

```glsl
// apps/explorer/src/scenes/terrain/terrain.frag.glsl
precision highp float;

// Palette
uniform vec3 uColorDeep;   // #0a0f1a -- valleys
uniform vec3 uColorMid;    // #3a2030 -- rose-deep
uniform vec3 uColorHigh;   // #dca5bd -- rose-glow
uniform vec3 uColorPeak;   // #d8c8a0 -- bone-bright

// Fog
uniform vec3 uFogColor;    // #060608 -- void
uniform float uFogDensity;

// Hash-derived modulation
uniform float uHueShift;   // 0-1
uniform float uSaturation; // 0-1
uniform float uOpacity;

varying float vHeight;
varying float vFogDepth;
varying vec2 vUv;

// ── Hue rotation (Rodrigues) ──
vec3 hueRotate(vec3 color, float shift) {
  // shift in [0, 1], mapped to +/- 0.15 turns = +/- 54 degrees
  float angle = (shift - 0.5) * 0.3 * 6.2832;
  float s = sin(angle);
  float c = cos(angle);
  vec3 w = vec3(0.299, 0.587, 0.114);
  float dot_val = dot(color, w);
  vec3 grey = vec3(dot_val);
  vec3 rotated;
  rotated.r = dot_val + (color.r - dot_val) * c + (0.168 * color.r - 0.131 * color.g - 0.037 * color.b) * s;
  rotated.g = dot_val + (color.g - dot_val) * c + (0.330 * color.r + 0.174 * color.g - 0.504 * color.b) * s;
  rotated.b = dot_val + (color.b - dot_val) * c + (-0.497 * color.r + 0.429 * color.g + 0.068 * color.b) * s;
  return rotated;
}

void main() {
  // ── Height-based color gradient ──
  // 4-stop gradient: deep -> mid -> high -> peak
  vec3 color;
  if (vHeight < 0.33) {
    color = mix(uColorDeep, uColorMid, vHeight / 0.33);
  } else if (vHeight < 0.66) {
    color = mix(uColorMid, uColorHigh, (vHeight - 0.33) / 0.33);
  } else {
    color = mix(uColorHigh, uColorPeak, (vHeight - 0.66) / 0.34);
  }

  // ── Hash-derived hue rotation ──
  color = hueRotate(color, uHueShift);

  // ── Saturation modulation ──
  float grey = dot(color, vec3(0.299, 0.587, 0.114));
  color = mix(vec3(grey), color, 0.6 + uSaturation * 0.4);

  // ── Simple directional shading ──
  // Approximate normal from height derivatives via UV
  // (dFdx/dFdy give screen-space derivatives for cheap normal estimation)
  vec3 dx = dFdx(vec3(vUv.x, vHeight * 2.0, vUv.y));
  vec3 dy = dFdy(vec3(vUv.x, vHeight * 2.0, vUv.y));
  vec3 normal = normalize(cross(dx, dy));
  vec3 lightDir = normalize(vec3(-0.4, 0.8, 0.4));
  float diffuse = max(dot(normal, lightDir), 0.0);
  color *= 0.4 + diffuse * 0.6;

  // ── Exponential fog ──
  float fogFactor = 1.0 - exp(-uFogDensity * vFogDepth * vFogDensity * vFogDepth);
  color = mix(color, uFogColor, clamp(fogFactor, 0.0, 1.0));

  gl_FragColor = vec4(color, uOpacity);
}
```

### Transaction Beam Markers

When a block contains transactions, vertical beams of light rise from the terrain tile.

```typescript
// apps/explorer/src/scenes/terrain/TransactionBeams.tsx

import { useMemo, useRef } from 'react';
import { useFrame } from '@react-three/fiber';
import * as THREE from 'three';
import type { TerrainRing } from './TerrainRing';
import type { ChainTransaction } from '@/data/types';

interface Props {
  ring: TerrainRing;
}

/**
 * Derive a beam position on the tile from a transaction hash.
 * Uses first 4 bytes for X offset, next 4 for Z offset within the tile.
 */
function txToBeamPosition(
  txHash: `0x${string}`,
  tileZ: number,
): [number, number, number] {
  const x = parseInt(txHash.slice(2, 10), 16);
  const z = parseInt(txHash.slice(10, 18), 16);
  const nx = ((x % 1000) / 1000 - 0.5) * 6; // within tile width (8 units, with margin)
  const nz = ((z % 1000) / 1000 - 0.5) * 6;
  return [nx, 0, tileZ + nz];
}

/**
 * Beam color: bone-bright (#d8c8a0) for value transfers,
 * rose-bright (#cc90a8) for contract creations.
 */
function txToBeamColor(tx: ChainTransaction): THREE.Color {
  if (tx.to === null) {
    return new THREE.Color(0xcc90a8); // contract creation
  }
  return new THREE.Color(0xd8c8a0); // value transfer
}

/**
 * Renders additive-blended vertical beam quads for transactions
 * on visible terrain tiles.
 */
export function TransactionBeams({ ring }: Props) {
  const groupRef = useRef<THREE.Group>(null!);

  // Beam material: additive blending for volumetric glow
  const beamMaterial = useMemo(
    () =>
      new THREE.MeshBasicMaterial({
        color: 0xd8c8a0,
        transparent: true,
        opacity: 0.6,
        blending: THREE.AdditiveBlending,
        side: THREE.DoubleSide,
        depthWrite: false,
      }),
    [],
  );

  // Beam geometry: narrow vertical quad
  const beamGeometry = useMemo(
    () => new THREE.PlaneGeometry(0.05, 1, 1, 1),
    [],
  );

  // Update beam visibility per frame (beams fade over 10 seconds)
  useFrame(() => {
    if (!groupRef.current) return;
    const now = performance.now();
    for (const child of groupRef.current.children) {
      const mesh = child as THREE.Mesh;
      const birthTime = mesh.userData.birthTime as number;
      const age = (now - birthTime) / 1000;
      const fadeProgress = Math.min(age / 10, 1); // fade over 10s
      (mesh.material as THREE.MeshBasicMaterial).opacity =
        0.6 * (1 - fadeProgress);
      if (fadeProgress >= 1) {
        mesh.visible = false;
      }
    }
  });

  return <group ref={groupRef} />;
}
```

---

## 5.4 Ring Buffer Manager

Manages a fixed-size pool of terrain tile slots. As new blocks arrive, the oldest tile beyond the fog boundary is recycled for the new block.

- [ ] `TerrainRing` class managing 10 tile slots
- [ ] `advance(block)` -- generate heightmap, push new tile, recycle oldest
- [ ] `update(delta)` -- smooth scroll animation (tiles move toward camera)
- [ ] Tile recycling: reuse geometry objects, just update heightmap data texture
- [ ] Smooth fade-in: new tiles enter at opacity 0, fade to 1 over 400ms
- [ ] Scroll speed matches block time: ~1 tile width per second
- [ ] Support for `prefers-reduced-motion`: skip scroll animation, place tiles instantly

```typescript
// apps/explorer/src/scenes/terrain/TerrainRing.ts

import { hashToHeightmap, extractParams } from './hashmap';
import type { TerrainParams } from './hashmap';
import type { ChainBlock, ChainTransaction } from '@/data/types';

const TILE_SPACING = 8.5; // Z distance between tile centers (8 tile + 0.5 gap)
const SCROLL_SPEED = 8.5; // world units per second (1 tile per second)
const FADE_IN_DURATION = 0.4; // seconds

export interface TerrainSlot {
  /** Stable identifier for React keys. */
  id: number;
  /** Whether this slot has a block assigned. */
  active: boolean;
  /** Block number (if active). */
  blockNumber: bigint;
  /** Block hash (if active). */
  blockHash: `0x${string}` | null;
  /** The 32x32 heightmap data. */
  heightmap: Float32Array | null;
  /** gasUsed / gasLimit, 0-1. */
  activityLevel: number;
  /** Hash-derived terrain parameters (hueShift, saturation, etc). */
  params: TerrainParams;
  /** Transactions in this block (for beam rendering). */
  transactions: ChainTransaction[];
  /** Current Z position in world space. */
  positionZ: number;
  /** Current opacity (0 = fading in, 1 = fully visible). */
  opacity: number;
  /** Time since this slot was assigned (seconds). Used for fade-in. */
  age: number;
}

const DEFAULT_PARAMS: TerrainParams = {
  baseElevation: 0.5,
  roughness: 0.5,
  ridgeFactor: 0.5,
  erosion: 0.5,
  hueShift: 0.5,
  saturation: 0.5,
  blendCurve: 0.5,
  features: 0.5,
};

export class TerrainRing {
  private slots: TerrainSlot[];
  private head = 0;              // index of newest slot
  private scrollOffset = 0;      // accumulated scroll distance
  private totalBlocksSeen = 0;
  private reducedMotion: boolean;

  constructor(private capacity: number = 10) {
    this.reducedMotion =
      typeof window !== 'undefined' &&
      window.matchMedia('(prefers-reduced-motion: reduce)').matches;

    this.slots = Array.from({ length: capacity }, (_, i) => ({
      id: i,
      active: false,
      blockNumber: 0n,
      blockHash: null,
      heightmap: null,
      activityLevel: 0,
      params: { ...DEFAULT_PARAMS },
      transactions: [],
      positionZ: -i * TILE_SPACING,
      opacity: 0,
      age: 0,
    }));
  }

  /** Get all slots (for rendering). */
  getSlots(): TerrainSlot[] {
    return this.slots;
  }

  /**
   * Advance the ring buffer with a new block.
   * Recycles the oldest slot and assigns it to the new block.
   */
  advance(block: ChainBlock): void {
    this.totalBlocksSeen++;

    // Pick the slot to recycle: the one farthest behind the camera
    const recycleIdx = this.head;
    this.head = (this.head + 1) % this.capacity;

    const slot = this.slots[recycleIdx];

    // Generate heightmap (expensive -- could be deferred to a worker)
    const heightmap = hashToHeightmap(block.hash, 32);
    const params = extractParams(block.hash);

    // Assign data to recycled slot
    slot.active = true;
    slot.blockNumber = block.number;
    slot.blockHash = block.hash;
    slot.heightmap = heightmap;
    slot.activityLevel = block._activityLevel;
    slot.params = params;
    slot.transactions = block.transactions;
    slot.age = 0;
    slot.opacity = this.reducedMotion ? 1 : 0; // instant if reduced motion

    // Position new tile at the far end (emerging from fog)
    slot.positionZ = -(this.totalBlocksSeen * TILE_SPACING) + this.scrollOffset;
  }

  /**
   * Called every frame. Scrolls all tiles toward the camera
   * and handles fade-in animation.
   */
  update(delta: number): void {
    if (this.reducedMotion) {
      // No scroll animation -- tiles snap to position
      return;
    }

    // Accumulate scroll
    this.scrollOffset += SCROLL_SPEED * delta;

    for (const slot of this.slots) {
      if (!slot.active) continue;

      // Update Z position (scroll toward camera)
      slot.positionZ += SCROLL_SPEED * delta;

      // Fade-in over FADE_IN_DURATION
      slot.age += delta;
      if (slot.age < FADE_IN_DURATION) {
        slot.opacity = slot.age / FADE_IN_DURATION;
      } else {
        slot.opacity = 1;
      }

      // Fade-out when tile has scrolled past the camera
      if (slot.positionZ > 15) {
        slot.opacity = Math.max(0, 1 - (slot.positionZ - 15) / 5);
      }
    }
  }

  /** Get the visible slot count (active and opacity > 0). */
  visibleCount(): number {
    return this.slots.filter((s) => s.active && s.opacity > 0).length;
  }
}
```

---

## 5.5 Block Interaction

Raycasting on terrain tile meshes for hover and click interaction.

- [ ] `Raycaster` checks pointer intersections against terrain meshes
- [ ] Hover: glow outline effect, show block number tooltip (glass panel style)
- [ ] Click: emit `entity:select` on the EventBus with block number, open BlockDetail panel
- [ ] Camera animation: smooth zoom toward selected block tile over 500ms
- [ ] Tooltip: positioned above intersection point, clamped to viewport bounds
- [ ] `Escape` key or click-away dismisses selection

```typescript
// apps/explorer/src/scenes/terrain/interactions.ts

import * as THREE from 'three';
import { bus } from '@/data/bus';
import { useUIStore } from '@/data/store';
import type { TerrainSlot } from './TerrainRing';

const raycaster = new THREE.Raycaster();
const pointer = new THREE.Vector2();

export interface TerrainHitResult {
  slot: TerrainSlot;
  point: THREE.Vector3;
  uv: THREE.Vector2;
}

/**
 * Perform a raycast against all terrain tile meshes.
 * Returns the nearest hit, or null if no tile was intersected.
 */
export function raycastTerrain(
  event: PointerEvent,
  camera: THREE.Camera,
  meshes: THREE.Mesh[],
  slots: TerrainSlot[],
): TerrainHitResult | null {
  // Normalize pointer to NDC [-1, 1]
  const rect = (event.target as HTMLElement).getBoundingClientRect();
  pointer.x = ((event.clientX - rect.left) / rect.width) * 2 - 1;
  pointer.y = -((event.clientY - rect.top) / rect.height) * 2 + 1;

  raycaster.setFromCamera(pointer, camera);
  const intersections = raycaster.intersectObjects(meshes, false);

  if (intersections.length === 0) return null;

  const hit = intersections[0];
  const meshIndex = meshes.indexOf(hit.object as THREE.Mesh);
  if (meshIndex < 0) return null;

  return {
    slot: slots[meshIndex],
    point: hit.point,
    uv: hit.uv ?? new THREE.Vector2(0.5, 0.5),
  };
}

/**
 * Handle hover state: sets a glow effect on the hovered tile and
 * shows a tooltip with the block number.
 */
export function handleTerrainHover(
  hit: TerrainHitResult | null,
  hoveredSlot: { current: TerrainSlot | null },
): { blockNumber: bigint; screenPos: { x: number; y: number } } | null {
  // Clear previous hover
  if (hoveredSlot.current && hoveredSlot.current !== hit?.slot) {
    // Reset glow on previous tile (set emissive to black)
    hoveredSlot.current = null;
  }

  if (!hit || !hit.slot.active) return null;

  hoveredSlot.current = hit.slot;

  return {
    blockNumber: hit.slot.blockNumber,
    screenPos: { x: 0, y: 0 }, // projected from hit.point in the component
  };
}

/**
 * Handle click: navigate to block detail view.
 */
export function handleTerrainClick(hit: TerrainHitResult | null): void {
  if (!hit || !hit.slot.active) return;

  bus.emit('entity:select' as any, {
    type: 'block',
    number: hit.slot.blockNumber,
  });

  useUIStore.getState().openDetail({
    type: 'block',
    number: hit.slot.blockNumber,
  });
}

/**
 * Smoothly animate the camera toward a selected terrain tile.
 * Lerps position and lookAt over 500ms.
 */
export function animateCameraToBlock(
  camera: THREE.PerspectiveCamera,
  targetZ: number,
  onComplete: () => void,
): { update: (delta: number) => boolean } {
  const startPos = camera.position.clone();
  const endPos = new THREE.Vector3(0, 6, targetZ + 8);
  const startLookAt = new THREE.Vector3(0, 0, -4);
  const endLookAt = new THREE.Vector3(0, 0, targetZ);
  let elapsed = 0;
  const duration = 0.5; // 500ms

  return {
    update(delta: number): boolean {
      elapsed += delta;
      const t = Math.min(elapsed / duration, 1);
      // Expo ease-out: t -> 1 - (1-t)^3
      const eased = 1 - Math.pow(1 - t, 3);

      camera.position.lerpVectors(startPos, endPos, eased);
      const lookAt = new THREE.Vector3().lerpVectors(startLookAt, endLookAt, eased);
      camera.lookAt(lookAt);

      if (t >= 1) {
        onComplete();
        return true; // animation complete
      }
      return false;
    },
  };
}
```

### Hover Tooltip Component

```typescript
// Inline within TerrainScene.tsx as an HTML overlay

import { Html } from '@react-three/drei';

interface TooltipProps {
  blockNumber: bigint;
  position: [number, number, number];
}

function TerrainTooltip({ blockNumber, position }: TooltipProps) {
  return (
    <Html position={position} center style={{ pointerEvents: 'none' }}>
      <div
        style={{
          background: 'rgba(8, 8, 12, 0.85)',
          backdropFilter: 'blur(12px)',
          border: '1px solid rgba(255, 255, 255, 0.07)',
          padding: '4px 10px',
          fontFamily: "'JetBrains Mono', monospace",
          fontSize: '10px',
          color: '#c8b8c0',
          textTransform: 'uppercase',
          letterSpacing: '0.28em',
          whiteSpace: 'nowrap',
        }}
      >
        BLOCK {blockNumber.toLocaleString()}
      </div>
    </Html>
  );
}
```

---

## 5.6 Performance Budget

Strict budgets to maintain 60fps on an M1 MacBook Air (integrated GPU, 8GB RAM).

| Metric | Budget | Measured by |
|--------|--------|-------------|
| Draw calls per frame | <= 10 | `renderer.info.render.calls` |
| Vertices per tile | 1,024 (32x32) | geometry.attributes.position.count |
| Total vertices (8 tiles) | <= 8,192 | sum of visible tile vertices |
| Total vertices (10 slots) | <= 10,240 | all allocated slots |
| Triangles per tile | ~1,922 (31x31x2) | geometry.index.count / 3 |
| Total triangles | <= 15,376 | 8 visible tiles |
| Frame time (scene render) | <= 6ms | `performance.now()` delta |
| Frame time (total) | <= 16.6ms | requestAnimationFrame budget |
| GPU memory (terrain) | ~5MB | tile textures + geometry |
| JS heap (heightmaps) | ~320KB | 10 slots x 32KB each |

### LOD Strategy

- [ ] On `'light'` performance tier: reduce grid to 16x16 (256 vertices/tile)
- [ ] On `'standard'` tier: use 32x32 (1024 vertices/tile) -- default
- [ ] On `'full'` tier: optionally 64x64 (4096 vertices/tile) for close-up detail

```typescript
// apps/explorer/src/scenes/terrain/lod.ts

import { detectTier } from '@/utils/performance';

export function getTerrainGridSize(): number {
  const tier = detectTier();
  switch (tier) {
    case 'light':
    case 'mobile':
      return 16;
    case 'standard':
      return 32;
    case 'full':
      return 32; // 64 available but not needed at default zoom
    default:
      return 32;
  }
}

export function getTerrainRingCapacity(): number {
  const tier = detectTier();
  switch (tier) {
    case 'light':
    case 'mobile':
      return 6;    // fewer tiles, lower memory
    case 'standard':
      return 10;
    case 'full':
      return 14;   // more tiles visible before fog
    default:
      return 10;
  }
}
```

### Heightmap Computation Offloading

On capable browsers, move `hashToHeightmap` to a Web Worker to keep the main thread free during block arrival.

```typescript
// apps/explorer/src/scenes/terrain/heightmapWorker.ts
//
// Web Worker entry point. Receives block hash, returns Float32Array.

import { hashToHeightmap, extractParams } from './hashmap';

self.onmessage = (e: MessageEvent<{ hash: `0x${string}`; gridSize: number }>) => {
  const { hash, gridSize } = e.data;
  const heightmap = hashToHeightmap(hash, gridSize);
  const params = extractParams(hash);
  // Transfer the ArrayBuffer for zero-copy
  self.postMessage(
    { heightmap, params },
    { transfer: [heightmap.buffer] },
  );
};
```

```typescript
// apps/explorer/src/scenes/terrain/useHeightmapWorker.ts

let worker: Worker | null = null;

function getWorker(): Worker {
  if (!worker) {
    worker = new Worker(
      new URL('./heightmapWorker.ts', import.meta.url),
      { type: 'module' },
    );
  }
  return worker;
}

/**
 * Compute a heightmap off the main thread.
 * Falls back to synchronous computation if Workers are unavailable.
 */
export function computeHeightmapAsync(
  hash: `0x${string}`,
  gridSize: number,
): Promise<{ heightmap: Float32Array; params: import('./hashmap').TerrainParams }> {
  return new Promise((resolve) => {
    try {
      const w = getWorker();
      const handler = (e: MessageEvent) => {
        w.removeEventListener('message', handler);
        resolve(e.data);
      };
      w.addEventListener('message', handler);
      w.postMessage({ hash, gridSize });
    } catch {
      // Fallback: synchronous
      const { hashToHeightmap, extractParams } = require('./hashmap');
      resolve({
        heightmap: hashToHeightmap(hash, gridSize),
        params: extractParams(hash),
      });
    }
  });
}
```

---

## 5.7 Verification

Acceptance criteria for the terrain scene. Each item must pass before the scene is considered complete.

- [ ] **Determinism**: same block hash always produces identical terrain. Run `hashToHeightmap` 100 times with the same hash; all outputs must be byte-identical.
- [ ] **Visual variety**: render 10 consecutive blocks; terrain tiles must show visible height variation (no two tiles look the same).
- [ ] **Fog emergence**: new blocks appear smoothly from fog at the far edge (opacity 0 to 1 over 400ms). Camera tracks smoothly.
- [ ] **Empty block rendering**: blocks with 0 transactions render as low-relief (max height 0.6) ghost-opacity terrain.
- [ ] **Active block rendering**: blocks with transactions render with dramatic relief (max height 2.0) and visible beam markers.
- [ ] **Click interaction**: clicking a terrain tile shows the BlockDetail panel for that block number.
- [ ] **Hover tooltip**: hovering over a tile shows a glass tooltip with the block number.
- [ ] **Performance (M1 MacBook Air)**: solid 60fps with 8 visible tiles. Frame time p95 < 10ms.
- [ ] **Performance (low-end)**: on `'light'` tier, grid reduces to 16x16 and ring reduces to 6 tiles. Still 30fps.
- [ ] **Reduced motion**: when `prefers-reduced-motion: reduce` is active, terrain does not scroll. Tiles appear instantly at their final position. No wave animation on vertices.
- [ ] **WebGL context loss recovery**: if the WebGL context is lost, show a fallback message and attempt restoration. If restoration fails, fall back to MOSAIC mode.
- [ ] **State root color shift**: when a block's `stateRoot` differs from its parent, the tile's hue shift is visibly different from the previous tile (tests the `hueShift` parameter variation).

### Test Fixtures

```typescript
// apps/explorer/src/scenes/terrain/__tests__/hashmap.test.ts

import { describe, it, expect } from 'vitest';
import { hashToHeightmap, extractParams } from '../hashmap';

const KNOWN_HASH =
  '0x6d1fb175dce4b6795c9eef1f47e6d4b3ae7e0cfd8c5f2b3a4d6e8f0a1c2b3d4e' as `0x${string}`;

describe('hashToHeightmap', () => {
  it('is deterministic', () => {
    const a = hashToHeightmap(KNOWN_HASH, 32);
    const b = hashToHeightmap(KNOWN_HASH, 32);
    expect(a).toEqual(b);
  });

  it('produces values in [0, 1]', () => {
    const heights = hashToHeightmap(KNOWN_HASH, 32);
    for (let i = 0; i < heights.length; i++) {
      expect(heights[i]).toBeGreaterThanOrEqual(0);
      expect(heights[i]).toBeLessThanOrEqual(1);
    }
  });

  it('produces 32*32 = 1024 values for gridSize=32', () => {
    const heights = hashToHeightmap(KNOWN_HASH, 32);
    expect(heights.length).toBe(1024);
  });

  it('different hashes produce different heightmaps', () => {
    const OTHER_HASH =
      '0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa' as `0x${string}`;
    const a = hashToHeightmap(KNOWN_HASH, 32);
    const b = hashToHeightmap(OTHER_HASH, 32);
    // At least 90% of values should differ
    let diffCount = 0;
    for (let i = 0; i < a.length; i++) {
      if (Math.abs(a[i] - b[i]) > 0.01) diffCount++;
    }
    expect(diffCount / a.length).toBeGreaterThan(0.9);
  });
});

describe('extractParams', () => {
  it('is deterministic', () => {
    const a = extractParams(KNOWN_HASH);
    const b = extractParams(KNOWN_HASH);
    expect(a).toEqual(b);
  });

  it('all values are in [0, 1]', () => {
    const p = extractParams(KNOWN_HASH);
    for (const key of Object.keys(p)) {
      const val = p[key as keyof typeof p];
      expect(val).toBeGreaterThanOrEqual(0);
      expect(val).toBeLessThanOrEqual(1);
    }
  });
});
```
