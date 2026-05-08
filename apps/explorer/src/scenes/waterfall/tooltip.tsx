import { useState, useEffect, type MutableRefObject } from 'react';
import type { ChainBlock } from '@/data/types';

export interface TooltipData {
  block: ChainBlock;
  x: number; // clientX
  y: number; // clientY
  barY: number; // Y position of the bar in canvas
}

interface Props {
  setRef: MutableRefObject<(data: TooltipData | null) => void>;
}

export function WaterfallTooltip({ setRef }: Props) {
  const [data, setData] = useState<TooltipData | null>(null);

  useEffect(() => {
    setRef.current = setData;
  }, [setRef]);

  if (!data) return null;

  const { block, x, y } = data;
  const gasRatio = Number(block.gasUsed) / Number(block.gasLimit);
  const age = Date.now() - block._arrivalTime;
  const ageSec = (age / 1000).toFixed(1);

  return (
    <div
      style={{
        position: 'fixed',
        left: x + 12,
        top: y - 60,
        background: 'rgba(8, 8, 12, 0.85)',
        backdropFilter: 'blur(12px)',
        border: '1px solid rgba(255, 255, 255, 0.07)',
        padding: '8px 12px',
        fontFamily: '"JetBrains Mono", monospace',
        fontSize: '10px',
        color: '#c8b8c0',
        pointerEvents: 'none',
        zIndex: 200,
        lineHeight: 1.6,
        whiteSpace: 'nowrap',
      }}
    >
      <div style={{ color: '#dca5bd', marginBottom: 2 }}>
        BLOCK {Number(block.number).toLocaleString()}
      </div>
      <div style={{ color: '#6a5a68', letterSpacing: '0.06em' }}>
        {block.hash.slice(0, 18)}...{block.hash.slice(-8)}
      </div>
      <div>
        GAS {(gasRatio * 100).toFixed(1)}%
        <span style={{ color: '#6a5a68' }}>
          {' '}({formatGasCompact(Number(block.gasUsed))} / {formatGasCompact(Number(block.gasLimit))})
        </span>
      </div>
      <div>
        {block.transactions.length} TXN
        <span style={{ color: '#6a5a68' }}> {ageSec}s ago</span>
      </div>
    </div>
  );
}

function formatGasCompact(gas: number): string {
  if (gas >= 1e9) return `${(gas / 1e9).toFixed(1)}B`;
  if (gas >= 1e6) return `${(gas / 1e6).toFixed(1)}M`;
  if (gas >= 1e3) return `${(gas / 1e3).toFixed(0)}K`;
  return gas.toString();
}
