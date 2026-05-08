import type { ChainBlock, FeeDataPoint } from '@/data/types';

/** Waveform configuration */
const WAVEFORM_HEIGHT = 120; // px logical height
const POINT_COUNT = 64; // blocks to show
const LINE_COLOR = '#aa7088'; // --rd-rose
const FILL_TOP = 'rgba(170, 112, 136, 0.10)'; // --rd-rose at 10%
const FILL_BOTTOM = 'rgba(170, 112, 136, 0.0)'; // transparent
const BASE_FEE_COLOR = '#c89a68'; // --rd-warning (burnt amber)
const GRID_COLOR = 'rgba(255, 255, 255, 0.03)';
const LABEL_COLOR = '#6a5a68'; // --rd-text-dim
const FONT = '9px "JetBrains Mono", monospace';

interface WaveformState {
  gasUsedHistory: number[];
  gasLimitHistory: number[];
  baseFeeHistory: number[];
  blockNumbers: bigint[];
}

/**
 * GasWaveform renderer.
 *
 * Uses OffscreenCanvas if available, falls back to regular canvas.
 * Redraws only when new data arrives (not every frame).
 */
export class GasWaveformRenderer {
  private canvas: HTMLCanvasElement;
  private ctx: CanvasRenderingContext2D;
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
    this.ctx = canvas.getContext('2d')!;
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

  /** Resolve a mouse X position to a data point */
  hitTest(clientX: number): { blockNumber: bigint; gasUsed: number; baseFee: number } | null {
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

  /** Full redraw of the waveform */
  private redraw(): void {
    const ctx = this.ctx;
    const w = this.width;
    const h = this.height;
    const s = this.state;

    if (w === 0 || s.gasUsedHistory.length < 2) return;

    ctx.clearRect(0, 0, w, h);

    // Margins
    const marginLeft = 48;
    const marginRight = 12;
    const marginTop = 8;
    const marginBottom = 20;
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

    // X-axis labels (block numbers at regular intervals)
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
