import { useState } from 'react';
import { useNavigate } from 'react-router';
import { AnimatePresence, motion } from 'framer-motion';
import { useChainStore } from '@/data/store';
import { truncateHash, formatGas } from '@/lib/format';
import { PanelHeader } from '@/panels/PanelHeader';
import { BlockHeatstrip } from '@/panels/charts/BlockHeatstrip';

function GasBar({ ratio }: { ratio: number }) {
  const pct = Math.min(100, Math.max(0, ratio * 100));
  const color =
    pct > 80 ? '#e05555' : pct > 50 ? '#e8a84c' : '#aa7088';

  return (
    <div
      style={{
        width: 100,
        height: 7,
        background: 'rgba(255,255,255,0.06)',
        borderRadius: 2,
        overflow: 'hidden',
        flexShrink: 0,
      }}
    >
      <div
        style={{
          width: `${Math.max(2, pct)}%`,
          height: '100%',
          background: color,
          transition: 'width 300ms ease-out',
        }}
      />
    </div>
  );
}

function timeAgo(arrivalTime: number): string {
  const elapsed = Math.max(0, Date.now() - arrivalTime);
  if (elapsed < 1000) return 'now';
  if (elapsed < 60_000) return `${Math.round(elapsed / 1000)}s ago`;
  return `${Math.round(elapsed / 60_000)}m ago`;
}

function accentColor(ratio: number): string {
  if (ratio >= 0.7) return '#cc5555';
  if (ratio >= 0.3) return '#c89a68';
  return '#7a8a78';
}

function BlockRow({ num, block }: { num: bigint; block: any }) {
  const navigate = useNavigate();
  const [hovered, setHovered] = useState(false);

  const gasRatio =
    block.gasLimit > 0n
      ? Number(block.gasUsed) / Number(block.gasLimit)
      : 0;

  return (
    <motion.div
      key={num.toString()}
      initial={{ opacity: 0, x: -20 }}
      animate={{ opacity: 1, x: 0 }}
      exit={{ opacity: 0, x: -20 }}
      transition={{ duration: 0.3, ease: [0.16, 1, 0.3, 1] }}
      style={{
        position: 'relative',
        display: 'grid',
        gridTemplateColumns: '1fr auto',
        gap: '3px 16px',
        padding: '10px 14px 10px 17px',
        borderBottom: '1px solid rgba(255,255,255,0.04)',
        fontFamily: 'var(--rd-font-mono)',
        cursor: 'pointer',
        background: hovered ? 'var(--rd-glass-bg-hover)' : 'transparent',
        transition: 'background 80ms ease-out',
      }}
      onMouseEnter={() => setHovered(true)}
      onMouseLeave={() => setHovered(false)}
      onClick={() => navigate('/block/' + Number(block.number))}
    >
      {/* Left accent bar */}
      <div
        style={{
          position: 'absolute',
          left: 0,
          top: 0,
          bottom: 0,
          width: 3,
          background: accentColor(gasRatio),
        }}
      />

      {/* Row 1: block number + time */}
      <div style={{ display: 'flex', alignItems: 'center', gap: 10 }}>
        <span
          style={{
            color: '#dca5bd',
            fontVariantNumeric: 'tabular-nums',
            fontSize: '14px',
            fontWeight: 600,
          }}
        >
          #{Number(block.number).toLocaleString()}
        </span>
        <span style={{ color: '#6a5a62', fontSize: '12px' }}>
          {truncateHash(block.hash, 5)}
        </span>
      </div>
      <span
        style={{
          color: '#8a7a82',
          textAlign: 'right',
          fontVariantNumeric: 'tabular-nums',
          fontSize: '12px',
        }}
      >
        {timeAgo(block._arrivalTime)}
      </span>

      {/* Row 2: tx count + gas bar */}
      <div
        style={{
          display: 'flex',
          alignItems: 'center',
          gap: 10,
          color: '#9a8a92',
          fontSize: '12px',
        }}
      >
        <span style={{ fontWeight: 500 }}>
          {block.transactions.length} txs
        </span>
        <GasBar ratio={gasRatio} />
        <span style={{ fontVariantNumeric: 'tabular-nums', color: '#7a6a72' }}>
          {formatGas(block.gasUsed)} gas
        </span>
      </div>
      <span
        style={{
          color: gasRatio > 0.5 ? '#e8a84c' : '#6a5a62',
          textAlign: 'right',
          fontVariantNumeric: 'tabular-nums',
          fontSize: '11px',
        }}
      >
        {(gasRatio * 100).toFixed(0)}%
      </span>
    </motion.div>
  );
}

export function BlockFeed() {
  const blockOrder = useChainStore((s) => s.blockOrder);
  const blocks = useChainStore((s) => s.blocks);

  const visibleBlocks = blockOrder.slice(0, 40);

  return (
    <div style={{ display: 'flex', flexDirection: 'column', overflow: 'hidden' }}>
      <PanelHeader icon="blocks" title="Latest Blocks" />
      <BlockHeatstrip />
      <div
        style={{
          overflowY: 'auto',
          flex: 1,
          scrollbarWidth: 'thin',
          scrollbarColor: 'rgba(170,112,136,0.3) transparent',
        }}
      >
        {visibleBlocks.length === 0 && (
          <div
            style={{
              padding: '32px 14px',
              fontFamily: 'var(--rd-font-mono)',
              fontSize: '13px',
              color: '#6a5a62',
              textAlign: 'center',
            }}
          >
            Waiting for blocks...
          </div>
        )}
        <AnimatePresence initial={false}>
          {visibleBlocks.map((num) => {
            const block = blocks.get(num);
            if (!block) return null;
            return <BlockRow key={num.toString()} num={num} block={block} />;
          })}
        </AnimatePresence>
      </div>
    </div>
  );
}
