/**
 * Render a block hash as a barcode pattern inside a rectangular area.
 *
 * Algorithm:
 * - Split the 32-byte hash into individual nibbles (64 nibbles)
 * - Each nibble (0-15) determines a bar's relative width
 * - Bars alternate between two subtle color variations
 * - The result is a unique visual fingerprint for every hash
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

  hashToBarcode(hash, width, height, ctx as unknown as CanvasRenderingContext2D);

  return ctx.getImageData(0, 0, width, height);
}
