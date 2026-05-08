import { clamp, lerp, mulberry32 } from '@/lib/math';

// ---- Utility ---------------------------------------------------------------

/** Decode a hex hash string into a Uint8Array of 32 bytes. */
function hexToBytes(hex: string): Uint8Array {
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

// ---- Seeded Perlin permutation table ----------------------------------------

/** Build a permutation table seeded by PRNG (for hash-unique noise). */
function buildPermTable(rng: () => number): Uint8Array {
  const perm = new Uint8Array(512);
  for (let i = 0; i < 256; i++) perm[i] = i;
  for (let i = 255; i > 0; i--) {
    const j = (rng() * (i + 1)) | 0;
    const tmp = perm[i];
    perm[i] = perm[j];
    perm[j] = tmp;
  }
  for (let i = 0; i < 256; i++) perm[i + 256] = perm[i];
  return perm;
}

const GRAD2: [number, number][] = [
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
function seededPerlin2D(x: number, y: number, perm: Uint8Array): number {
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

// ---- Erosion ----------------------------------------------------------------

function thermalErosion(
  heights: Float32Array,
  size: number,
  talusAngle: number,
): void {
  const transfer = 0.3;
  for (let y = 1; y < size - 1; y++) {
    for (let x = 1; x < size - 1; x++) {
      const i = y * size + x;
      const h = heights[i];

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

function hydraulicErosion(
  heights: Float32Array,
  size: number,
  rng: () => number,
): void {
  const droplets = size * 2;
  for (let d = 0; d < droplets; d++) {
    let x = (rng() * (size - 2) + 1) | 0;
    let y = (rng() * (size - 2) + 1) | 0;
    let sediment = 0;
    const erosionRate = 0.01;
    const depositionRate = 0.005;

    for (let step = 0; step < 16; step++) {
      const i = y * size + x;
      const h = heights[i];

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
        heights[i] += sediment * depositionRate;
        break;
      }

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

// ---- Terrain parameters -----------------------------------------------------

export interface TerrainParams {
  baseElevation: number;
  roughness: number;
  ridgeFactor: number;
  erosion: number;
  hueShift: number;
  saturation: number;
  blendCurve: number;
  features: number;
}

/** Extract terrain parameters from a 32-byte hash. */
export function extractParams(hash: string): TerrainParams {
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

// ---- Main heightmap generator -----------------------------------------------

/**
 * Convert a block hash into a heightmap.
 *
 * 5 noise layers at increasing frequency, seeded from hash byte groups.
 * Followed by an erosion pass (thermal + hydraulic).
 *
 * @param hash     - 0x-prefixed 32-byte hex block hash
 * @param gridSize - vertex resolution per axis (default 32)
 * @returns Float32Array of gridSize*gridSize heights in [0, 1]
 */
export function hashToHeightmap(
  hash: string,
  gridSize: number = 32,
): Float32Array {
  const bytes = hexToBytes(hash);
  const params = extractParams(hash);
  const total = gridSize * gridSize;
  const heights = new Float32Array(total);

  // Seed PRNG from first 4 bytes
  const rng = mulberry32(readUint32(bytes, 0));
  const perm = buildPermTable(rng);

  // Layer configurations derived from hash byte groups
  const layers = [
    { amp: 1.0, freq: 0.02, seedOffset: readUint32(bytes, 0) },
    { amp: 0.5, freq: 0.04, seedOffset: readUint32(bytes, 8) },
    { amp: 0.25, freq: 0.08, seedOffset: readUint32(bytes, 16) },
    { amp: 0.125, freq: 0.16, seedOffset: readUint32(bytes, 24) },
  ];

  for (let y = 0; y < gridSize; y++) {
    for (let x = 0; x < gridSize; x++) {
      const i = y * gridSize + x;
      const nx = x / (gridSize - 1);
      const ny = y / (gridSize - 1);

      let h = 0;
      let ampSum = 0;

      for (const layer of layers) {
        const sx = (nx + layer.seedOffset * 0.001) * gridSize * layer.freq;
        const sy = (ny + layer.seedOffset * 0.0007) * gridSize * layer.freq;
        h += seededPerlin2D(sx, sy, perm) * layer.amp;
        ampSum += layer.amp;
      }

      // Normalize to [0, 1]
      h = (h / ampSum) * 0.5 + 0.5;

      // Ridge layer
      const ridgeNx = nx * gridSize * 0.03 + readUint32(bytes, 8) * 0.001;
      const ridgeNy = ny * gridSize * 0.03 + readUint32(bytes, 12) * 0.001;
      const ridge = 1.0 - Math.abs(seededPerlin2D(ridgeNx, ridgeNy, perm));
      h = lerp(h, ridge, params.ridgeFactor * 0.6);

      // Base elevation offset
      h = h * 0.7 + params.baseElevation * 0.3;

      // Feature placement
      const featureX = params.features * (gridSize - 1);
      const featureY = (1 - params.features) * (gridSize - 1);
      const distToFeature = Math.hypot(x - featureX, y - featureY) / gridSize;
      h += Math.max(0, 0.3 - distToFeature) * params.features;

      heights[i] = clamp(h, 0, 1);
    }
  }

  // Erosion pass
  if (params.erosion > 0.2) {
    const talusAngle = 0.05 + (1 - params.erosion) * 0.1;
    for (let iter = 0; iter < 2; iter++) {
      thermalErosion(heights, gridSize, talusAngle);
    }
    hydraulicErosion(heights, gridSize, rng);
  }

  // Normalize to [0, 1] post-erosion
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

// ---- Color palette ----------------------------------------------------------

export const TERRAIN_PALETTE = {
  deep: { r: 0x0a / 255, g: 0x0f / 255, b: 0x1a / 255 },
  mid: { r: 0x3a / 255, g: 0x20 / 255, b: 0x30 / 255 },
  high: { r: 0xdc / 255, g: 0xa5 / 255, b: 0xbd / 255 },
  peak: { r: 0xd8 / 255, g: 0xc8 / 255, b: 0xa0 / 255 },
} as const;

/** Map a height value [0, 1] to an RGB color triple. */
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

  // Hue rotation
  const angle = (hueShift - 0.5) * 0.3 * Math.PI * 2;
  const cos = Math.cos(angle);
  const sin = Math.sin(angle);
  const rr =
    r * (0.299 + 0.701 * cos) +
    g * (0.587 - 0.587 * cos) +
    b * (0.114 - 0.114 * cos);
  const gg =
    r * (0.299 - 0.299 * cos + 0.328 * sin) +
    g * (0.587 + 0.413 * cos) +
    b * (0.114 - 0.114 * cos - 0.328 * sin);
  const bb =
    r * (0.299 - 0.299 * cos - 1.25 * sin) +
    g * (0.587 - 0.587 * cos + 1.05 * sin) +
    b * (0.114 + 0.886 * cos + 0.203 * sin);

  return [clamp(rr, 0, 1), clamp(gg, 0, 1), clamp(bb, 0, 1)];
}
