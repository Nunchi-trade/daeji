/**
 * Hash Art Generator
 *
 * Converts a 0x-prefixed hex hash (64 hex chars) into a deterministic
 * visual pattern on a canvas.
 *
 * Algorithm:
 * 1. Parse hash into 32 bytes.
 * 2. Bytes 0-2: derive 3-color palette (hue rotation within ROSEDUST spectrum).
 * 3. Byte 3: select pattern algorithm (8 possible patterns via modulo).
 * 4. Bytes 4-19: fill an 8x4 half-grid (4 bits per byte, 16 bytes = 64 cells / 2).
 * 5. Mirror horizontally for bilateral symmetry.
 * 6. Byte 20: density threshold -- cells above threshold are filled, below are void.
 * 7. Render each filled cell as a colored rectangle on the canvas.
 */

// ROSEDUST hue anchors (HSL hue values for palette generation)
const HUE_ROSE = 340;
const HUE_BONE = 40;
const HUE_DREAM = 240;
const HUE_ANCHORS = [HUE_ROSE, HUE_BONE, HUE_DREAM];

// ---------------------------------------------------------------------------
// Byte parsing
// ---------------------------------------------------------------------------

function hashToBytes(hash: `0x${string}`): Uint8Array {
  const hex = hash.slice(2);
  const bytes = new Uint8Array(32);
  for (let i = 0; i < 32; i++) {
    bytes[i] = parseInt(hex.slice(i * 2, i * 2 + 2), 16);
  }
  return bytes;
}

// ---------------------------------------------------------------------------
// Color palette
// ---------------------------------------------------------------------------

interface Palette {
  bg: string;
  colors: [string, string, string];
}

function derivePalette(bytes: Uint8Array): Palette {
  const hueShift = (bytes[0] / 255) * 30 - 15;
  const satBase = 30 + (bytes[1] / 255) * 30;
  const lightBase = 40 + (bytes[2] / 255) * 20;

  const colors = HUE_ANCHORS.map((hue) => {
    const h = (hue + hueShift + 360) % 360;
    return `hsl(${h}, ${satBase}%, ${lightBase}%)`;
  }) as [string, string, string];

  const bg = '#0a0a0e';
  return { bg, colors };
}

// ---------------------------------------------------------------------------
// Grid generation (8x8, mirrored)
// ---------------------------------------------------------------------------

type Grid = number[][]; // 8x8, values 0-2 (color index) or -1 (empty)

function generateGrid(bytes: Uint8Array): Grid {
  const grid: Grid = Array.from({ length: 8 }, () => Array(8).fill(-1));
  const threshold = bytes[3];

  for (let row = 0; row < 8; row++) {
    for (let col = 0; col < 4; col++) {
      const byteIndex = 4 + row * 2 + Math.floor(col / 2);
      const nibble =
        col % 2 === 0
          ? (bytes[byteIndex] >> 4) & 0x0f
          : bytes[byteIndex] & 0x0f;

      const value = nibble * 17; // 0-255
      if (value > threshold) {
        grid[row][col] = nibble % 3;
        grid[row][7 - col] = nibble % 3;
      }
    }
  }

  return grid;
}

// ---------------------------------------------------------------------------
// Render
// ---------------------------------------------------------------------------

export function generateHashArt(
  ctx: CanvasRenderingContext2D,
  hash: `0x${string}`,
  size: number,
): void {
  const bytes = hashToBytes(hash);
  const palette = derivePalette(bytes);
  const grid = generateGrid(bytes);

  const cellSize = size / 8;

  ctx.fillStyle = palette.bg;
  ctx.fillRect(0, 0, size, size);

  for (let row = 0; row < 8; row++) {
    for (let col = 0; col < 8; col++) {
      const colorIndex = grid[row][col];
      if (colorIndex === -1) continue;

      ctx.fillStyle = palette.colors[colorIndex];
      ctx.fillRect(col * cellSize, row * cellSize, cellSize, cellSize);
    }
  }
}
