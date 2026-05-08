import { useMemo } from 'react';
import { useChainStore } from '@/data/store';

function gasRatioColor(ratio: number): string {
  if (ratio < 0.25) return 'var(--rd-success)';
  if (ratio < 0.50) return 'var(--rd-warning)';
  if (ratio < 0.75) return 'var(--rd-warning)';
  return 'var(--rd-danger)';
}

function gasRatioOpacity(ratio: number): number {
  if (ratio < 0.50) return 0.3 + ratio * 0.6; // 0.3 at 0, ~0.6 at 0.5
  return 0.6 + (ratio - 0.5) * 0.8;           // 0.6 at 0.5, 1.0 at 1.0
}

export function BlockHeatstrip() {
  const blockOrder = useChainStore((s) => s.blockOrder);
  const blocks = useChainStore((s) => s.blocks);

  const cells = useMemo(() => {
    const slice = blockOrder.slice(0, 40).slice().reverse();
    return slice.map((num) => {
      const block = blocks.get(num);
      if (!block) return null;
      const gasLimit = Number(block.gasLimit);
      const ratio = gasLimit > 0 ? Number(block.gasUsed) / gasLimit : 0;
      const color = gasRatioColor(ratio);
      const opacity = gasRatioOpacity(Math.min(ratio, 1));
      return { num, ratio, color, opacity };
    });
  }, [blockOrder, blocks]);

  if (cells.length === 0) return null;

  return (
    <div style={{ padding: '0 14px 8px' }}>
      <div
        style={{
          display: 'flex',
          gap: 1,
          height: 12,
        }}
      >
        {cells.map((cell) => {
          if (!cell) return null;
          return (
            <div
              key={String(cell.num)}
              style={{
                flex: 1,
                height: 12,
                backgroundColor: cell.color,
                opacity: cell.opacity,
              }}
            />
          );
        })}
      </div>
    </div>
  );
}
