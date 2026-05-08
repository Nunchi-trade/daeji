import type { ChainBlock } from '@/data/types';
import { hashToBarcode } from './hashbar';
import { gasToColor } from './blockBar';

/**
 * Pure class representing a single block card in the waterfall.
 * Used for falling animation and stack compression calculations.
 */
export class BlockCard {
  readonly blockNumber: bigint;
  readonly hash: string;
  readonly gasUsed: number;
  readonly gasLimit: number;
  readonly txCount: number;
  readonly timestamp: number;
  readonly arrivalTime: number;

  /** Current Y position (animated) */
  y: number = -60;
  /** Current opacity (animated) */
  opacity: number = 0;
  /** Base height before compression */
  baseHeight: number;
  /** Current compressed height */
  height: number;
  /** Animation start time */
  animStartTime: number;

  private static readonly FALL_DURATION = 400; // ms
  private static readonly MAX_VISIBLE = 50;

  constructor(block: ChainBlock) {
    this.blockNumber = block.number;
    this.hash = block.hash;
    this.gasUsed = Number(block.gasUsed);
    this.gasLimit = Number(block.gasLimit);
    this.txCount = block.transactions.length;
    this.timestamp = Number(block.timestamp);
    this.arrivalTime = block._arrivalTime;
    this.animStartTime = Date.now();

    // Height based on gas utilization: 4px (empty) to 60px (full)
    const gasRatio = this.gasLimit > 0 ? this.gasUsed / this.gasLimit : 0;
    this.baseHeight = 4 + gasRatio * 56;
    this.height = this.baseHeight;
  }

  /** Gas usage ratio 0..1 */
  get gasRatio(): number {
    return this.gasLimit > 0 ? this.gasUsed / this.gasLimit : 0;
  }

  /** Update animation state. Returns true if still animating. */
  update(age: number): boolean {
    const elapsed = Date.now() - this.animStartTime;
    const t = Math.min(1, elapsed / BlockCard.FALL_DURATION);

    // Expo ease-out: 1 - 2^(-10t)
    const eased = t === 1 ? 1 : 1 - Math.pow(2, -10 * t);

    this.opacity = eased;

    // Compress based on age in the stack
    if (age < BlockCard.MAX_VISIBLE) {
      this.height = this.baseHeight * (1 - (age / BlockCard.MAX_VISIBLE) * 0.7);
    } else {
      this.height = this.baseHeight * 0.3;
    }

    return t < 1;
  }

  /** Draw this card onto a Canvas2D context */
  draw(
    ctx: CanvasRenderingContext2D,
    x: number,
    y: number,
    width: number,
  ): void {
    const h = Math.max(2, this.height);
    const gasRatio = this.gasRatio;

    ctx.save();
    ctx.globalAlpha = this.opacity;

    // Background fill (gas-colored)
    ctx.fillStyle = gasToColor(gasRatio);
    ctx.fillRect(x, y, width, h);

    // Hash barcode overlay
    if (width > 20 && h > 4) {
      hashToBarcode(this.hash, width, h, ctx, x, y);
    }

    // Left accent bar: rose-glow if block has txns, rose-dim otherwise
    const accentWidth = 2;
    ctx.fillStyle = this.txCount > 0 ? '#dca5bd' : '#3a2030';
    ctx.fillRect(x, y, accentWidth, h);

    // Block number label
    ctx.font = '10px "JetBrains Mono", monospace';
    ctx.textBaseline = 'middle';
    if (h >= 14) {
      ctx.textAlign = 'left';
      ctx.fillStyle = 'rgba(200, 184, 192, 0.7)'; // --rd-text at 70%
      ctx.fillText(
        Number(this.blockNumber).toLocaleString(),
        x + accentWidth + 6,
        y + h / 2,
      );
    }

    // Tx count (right side)
    if (this.txCount > 0 && h >= 14) {
      ctx.textAlign = 'right';
      ctx.fillStyle = '#d8c8a0'; // --rd-bone-bright
      ctx.fillText(
        `${this.txCount} tx`,
        x + width - 6,
        y + h / 2,
      );
    }

    // Bottom border
    ctx.fillStyle = 'rgba(255, 255, 255, 0.04)';
    ctx.fillRect(x, y + h - 1, width, 1);

    ctx.restore();
  }
}
