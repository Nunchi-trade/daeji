import { useMemo } from 'react';
import { useChainStore } from '@/data/store';

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

const CHART_WIDTH = 320;
const CHART_HEIGHT = 80;
const BAR_WIDTH = 60;
const BAR_GAP = 16;
const MAX_BAR_HEIGHT = 50;
const BAR_BOTTOM_Y = 60;
const NUM_BARS = 4;
const START_X = (CHART_WIDTH - NUM_BARS * BAR_WIDTH - (NUM_BARS - 1) * BAR_GAP) / 2;

const BUCKET_COLORS = [
  'var(--rd-success)',  // 0-25%
  'var(--rd-dream)',    // 25-50%
  'var(--rd-warning)', // 50-75%
  'var(--rd-danger)',  // 75-100%
];

const BUCKET_LABELS = ['0-25%', '25-50%', '50-75%', '75-100%'];

// ---------------------------------------------------------------------------
// Component
// ---------------------------------------------------------------------------

export function BlockFullness() {
  const blockOrder = useChainStore((s) => s.blockOrder);
  const blocks = useChainStore((s) => s.blocks);

  const buckets = useMemo(() => {
    const recent = blockOrder.slice(0, 40);
    const counts = [0, 0, 0, 0];

    for (const num of recent) {
      const block = blocks.get(num);
      if (!block) continue;

      const gasLimit = block.gasLimit;
      if (gasLimit === 0n) continue;

      const ratio = Number(block.gasUsed) / Number(gasLimit);

      if (ratio < 0.25) {
        counts[0]++;
      } else if (ratio < 0.5) {
        counts[1]++;
      } else if (ratio < 0.75) {
        counts[2]++;
      } else {
        counts[3]++;
      }
    }

    return counts;
  }, [blockOrder, blocks]);

  const hasData = blockOrder.length > 0;

  const maxCount = useMemo(() => Math.max(...buckets, 1), [buckets]);

  return (
    <div style={{ fontFamily: 'var(--rd-font-mono)' }}>
      {/* Header */}
      <div
        style={{
          padding: '10px 14px',
          fontFamily: 'var(--rd-font-mono)',
          fontSize: '12px',
          fontWeight: 600,
          textTransform: 'uppercase',
          letterSpacing: '0.1em',
          color: '#aa7088',
          borderBottom: '1px solid rgba(255,255,255,0.06)',
        }}
      >
        Block Fullness
      </div>

      {/* Chart area */}
      <div style={{ padding: '8px 14px' }}>
        {!hasData ? (
          <div
            style={{
              width: '100%',
              height: `${CHART_HEIGHT}px`,
              display: 'flex',
              alignItems: 'center',
              justifyContent: 'center',
              color: 'var(--rd-text-dim)',
              fontSize: '10px',
              fontFamily: 'var(--rd-font-mono)',
            }}
          >
            No block data
          </div>
        ) : (
          <svg
            width="100%"
            viewBox={`0 0 ${CHART_WIDTH} ${CHART_HEIGHT}`}
            aria-label="Block fullness histogram"
          >
            {buckets.map((count, i) => {
              const barHeight = (count / maxCount) * MAX_BAR_HEIGHT;
              const x = START_X + i * (BAR_WIDTH + BAR_GAP);
              const y = BAR_BOTTOM_Y - barHeight;
              const centerX = x + BAR_WIDTH / 2;

              return (
                <g key={i}>
                  {/* Bar */}
                  <rect
                    x={x}
                    y={y}
                    width={BAR_WIDTH}
                    height={barHeight}
                    fill={BUCKET_COLORS[i]}
                    rx={2}
                    ry={2}
                  />
                  {/* Clip bottom border-radius so only top has rounded corners */}
                  {barHeight > 2 && (
                    <rect
                      x={x}
                      y={BAR_BOTTOM_Y - 2}
                      width={BAR_WIDTH}
                      height={2}
                      fill={BUCKET_COLORS[i]}
                    />
                  )}

                  {/* Count label above bar */}
                  <text
                    x={centerX}
                    y={y - 3}
                    textAnchor="middle"
                    fill="var(--rd-text)"
                    fontSize={8}
                    fontFamily="var(--rd-font-mono)"
                  >
                    {count}
                  </text>

                  {/* Bucket label below bar */}
                  <text
                    x={centerX}
                    y={74}
                    textAnchor="middle"
                    fill="var(--rd-text-dim)"
                    fontSize={7}
                    fontFamily="var(--rd-font-mono)"
                  >
                    {BUCKET_LABELS[i]}
                  </text>
                </g>
              );
            })}
          </svg>
        )}
      </div>
    </div>
  );
}
