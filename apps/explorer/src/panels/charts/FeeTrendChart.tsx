import { useMemo } from 'react';
import { useChainStore } from '@/data/store';
import { toBezierPath, toAreaPath } from './svg-utils';

const VIEW_WIDTH = 320;
const VIEW_HEIGHT = 120;
const LEFT_MARGIN = 40;
const BOTTOM_MARGIN = 16;
const PLOT_X_START = LEFT_MARGIN;
const PLOT_X_END = VIEW_WIDTH;
const PLOT_Y_START = 4;
const PLOT_Y_END = VIEW_HEIGHT - BOTTOM_MARGIN; // 104

const GRADIENT_ID = 'feeTrend-fill';

export function FeeTrendChart() {
  const feeHistory = useChainStore((s) => s.feeHistory);

  const data = useMemo(() => {
    const points = feeHistory.slice(-64);
    if (points.length < 2) return null;

    const values = points.map((f) => Number(f.baseFee) / 1e9);
    const min = Math.min(...values);
    const max = Math.max(...values);
    const range = max - min || 1;
    const mid = min + range / 2;

    const plotWidth = PLOT_X_END - PLOT_X_START;
    const plotHeight = PLOT_Y_END - PLOT_Y_START;

    const svgPoints = values.map((v, i) => ({
      x: PLOT_X_START + (i / (values.length - 1)) * plotWidth,
      y: PLOT_Y_START + plotHeight - ((v - min) / range) * plotHeight,
    }));

    const linePath = toBezierPath(svgPoints);
    const areaPath = toAreaPath(svgPoints, PLOT_Y_END);

    const firstBlock = points[0].blockNumber;
    const lastBlock = points[points.length - 1].blockNumber;

    return {
      linePath,
      areaPath,
      minLabel: `${min.toFixed(2)}`,
      midLabel: `${mid.toFixed(2)}`,
      maxLabel: `${max.toFixed(2)}`,
      firstBlock: `#${firstBlock.toString()}`,
      lastBlock: `#${lastBlock.toString()}`,
    };
  }, [feeHistory]);

  if (!data) {
    return (
      <svg
        width="100%"
        viewBox={`0 0 ${VIEW_WIDTH} ${VIEW_HEIGHT}`}
        preserveAspectRatio="none"
        style={{ height: 120, display: 'block' }}
        aria-label="Fee trend (no data)"
      >
        <line
          x1={PLOT_X_START}
          y1={VIEW_HEIGHT / 2}
          x2={PLOT_X_END}
          y2={VIEW_HEIGHT / 2}
          stroke="var(--rd-text-dim)"
          strokeWidth={1}
          strokeDasharray="4 4"
        />
        <text
          x={(PLOT_X_START + PLOT_X_END) / 2}
          y={VIEW_HEIGHT / 2 - 6}
          textAnchor="middle"
          fill="var(--rd-text-dim)"
          fontSize={8}
          fontFamily="var(--rd-font-mono)"
        >
          No fee data
        </text>
      </svg>
    );
  }

  return (
    <svg
      width="100%"
      viewBox={`0 0 ${VIEW_WIDTH} ${VIEW_HEIGHT}`}
      preserveAspectRatio="none"
      style={{ height: 120, display: 'block' }}
      aria-label="Base fee trend"
    >
      <defs>
        <linearGradient id={GRADIENT_ID} x1="0" y1="0" x2="0" y2="1">
          <stop offset="0%" stopColor="var(--rd-warning)" stopOpacity={0.2} />
          <stop offset="100%" stopColor="var(--rd-warning)" stopOpacity={0} />
        </linearGradient>
      </defs>

      {/* Gradient fill */}
      <path d={data.areaPath} fill={`url(#${GRADIENT_ID})`} />

      {/* Line */}
      <path
        d={data.linePath}
        fill="none"
        stroke="var(--rd-warning)"
        strokeWidth={1.5}
        strokeLinecap="round"
        strokeLinejoin="round"
      />

      {/* Y-axis labels: max, mid, min */}
      <text
        x={36}
        y={PLOT_Y_START + 6}
        textAnchor="end"
        fill="var(--rd-text-dim)"
        fontSize={8}
        fontFamily="var(--rd-font-mono)"
        style={{ fontVariantNumeric: 'tabular-nums' }}
      >
        {data.maxLabel}
      </text>
      <text
        x={36}
        y={(PLOT_Y_START + PLOT_Y_END) / 2 + 3}
        textAnchor="end"
        fill="var(--rd-text-dim)"
        fontSize={8}
        fontFamily="var(--rd-font-mono)"
        style={{ fontVariantNumeric: 'tabular-nums' }}
      >
        {data.midLabel}
      </text>
      <text
        x={36}
        y={PLOT_Y_END - 1}
        textAnchor="end"
        fill="var(--rd-text-dim)"
        fontSize={8}
        fontFamily="var(--rd-font-mono)"
        style={{ fontVariantNumeric: 'tabular-nums' }}
      >
        {data.minLabel}
      </text>

      {/* X-axis: start and end block numbers */}
      <text
        x={PLOT_X_START}
        y={116}
        textAnchor="start"
        fill="var(--rd-text-dim)"
        fontSize={8}
        fontFamily="var(--rd-font-mono)"
        style={{ fontVariantNumeric: 'tabular-nums' }}
      >
        {data.firstBlock}
      </text>
      <text
        x={PLOT_X_END}
        y={116}
        textAnchor="end"
        fill="var(--rd-text-dim)"
        fontSize={8}
        fontFamily="var(--rd-font-mono)"
        style={{ fontVariantNumeric: 'tabular-nums' }}
      >
        {data.lastBlock}
      </text>
    </svg>
  );
}
