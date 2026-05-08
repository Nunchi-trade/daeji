import { useMemo } from 'react';
import { useChainStore } from '@/data/store';
import { formatGas } from '@/lib/format';
import { toBezierPath, toAreaPath } from './svg-utils';

interface GasUsageChartProps {
  width?: number;
  height?: number;
}

export function GasUsageChart({ width = 160, height = 48 }: GasUsageChartProps) {
  const blockOrder = useChainStore((s) => s.blockOrder);
  const blocks = useChainStore((s) => s.blocks);

  const { linePath, areaPath, totalGas } = useMemo(() => {
    const slice = blockOrder.slice(0, 20).slice().reverse();
    const values = slice.map((num) => {
      const block = blocks.get(num);
      return block ? Number(block.gasUsed) : 0;
    });

    const total = values.reduce((a, b) => a + b, 0);

    if (values.length < 2) {
      return { linePath: '', areaPath: '', totalGas: total };
    }

    const min = Math.min(...values);
    const max = Math.max(...values);
    const range = max - min || 1;

    const padY = 2;
    const usableHeight = height - padY * 2;

    const points = values.map((v, i) => ({
      x: (i / (values.length - 1)) * width,
      y: padY + usableHeight - ((v - min) / range) * usableHeight,
    }));

    return {
      linePath: toBezierPath(points),
      areaPath: toAreaPath(points, height),
      totalGas: total,
    };
  }, [blockOrder, blocks, width, height]);

  const gradientId = 'gasUsage-fill';

  if (!linePath) {
    return (
      <svg
        width={width}
        height={height}
        style={{ display: 'block' }}
        aria-label="Gas usage chart (no data)"
      >
        <line
          x1={0}
          y1={height / 2}
          x2={width}
          y2={height / 2}
          stroke="var(--rd-text-ghost)"
          strokeWidth={1}
          strokeDasharray="4 4"
        />
      </svg>
    );
  }

  return (
    <svg
      width={width}
      height={height}
      style={{ display: 'block' }}
      aria-label="Gas usage chart"
    >
      <defs>
        <linearGradient id={gradientId} x1="0" y1="0" x2="0" y2="1">
          <stop offset="0%" stopColor="var(--rd-rose)" stopOpacity={0.25} />
          <stop offset="100%" stopColor="var(--rd-rose)" stopOpacity={0} />
        </linearGradient>
      </defs>

      <path d={areaPath} fill={`url(#${gradientId})`} />

      <path
        d={linePath}
        fill="none"
        stroke="var(--rd-rose)"
        strokeWidth={1.5}
        strokeLinecap="round"
        strokeLinejoin="round"
      />

      <text
        x={width - 2}
        y={10}
        textAnchor="end"
        fill="var(--rd-text-dim)"
        fontSize={10}
        fontFamily="var(--rd-font-mono)"
        style={{ fontVariantNumeric: 'tabular-nums' }}
      >
        {formatGas(totalGas)}
      </text>
    </svg>
  );
}
