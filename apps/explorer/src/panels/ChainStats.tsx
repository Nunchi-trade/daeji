import { useChainStore } from '@/data/store';
import { AnimatedNumber } from '@/lib/AnimatedNumber';
import { PanelHeader } from '@/panels/PanelHeader';
import { BlockTimeSparkline } from '@/panels/charts/BlockTimeSparkline';
import { FeeTrendChart } from '@/panels/charts/FeeTrendChart';
import { GasUsageChart } from '@/panels/charts/GasUsageChart';
import { TxVolumeChart } from '@/panels/charts/TxVolumeChart';

function Stat({
  label,
  value,
  color,
}: {
  label: string;
  value: string;
  color?: string;
}) {
  return (
    <div style={{ padding: '10px 14px' }}>
      <div
        style={{
          fontFamily: 'var(--rd-font-mono)',
          fontSize: '11px',
          textTransform: 'uppercase',
          letterSpacing: '0.08em',
          color: '#8a7a82',
          marginBottom: 4,
        }}
      >
        {label}
      </div>
      <div
        style={{
          fontFamily: 'var(--rd-font-mono)',
          fontSize: '14px',
          color: color ?? '#d8c8c4',
          fontVariantNumeric: 'tabular-nums',
        }}
      >
        {value}
      </div>
    </div>
  );
}

const chartLabelStyle: React.CSSProperties = {
  fontFamily: 'var(--rd-font-mono)',
  fontSize: '10px',
  textTransform: 'uppercase',
  letterSpacing: '0.08em',
  color: '#8a7a82',
  padding: '8px 14px 4px',
};

export function ChainStats() {
  const latestBlock = useChainStore((s) => s.latestBlock);
  const blocks = useChainStore((s) => s.blocks);
  const blockOrder = useChainStore((s) => s.blockOrder);

  let totalTxs = 0;
  let avgGasRatio = 0;

  const recentBlocks = blockOrder.slice(0, 20);
  for (const num of recentBlocks) {
    const b = blocks.get(num);
    if (!b) continue;
    totalTxs += b.transactions.length;
    if (b.gasLimit > 0n) {
      avgGasRatio += Number(b.gasUsed) / Number(b.gasLimit);
    }
  }

  const avgGas =
    recentBlocks.length > 0
      ? ((avgGasRatio / recentBlocks.length) * 100).toFixed(1) + '%'
      : '--';

  const latestBlockData = latestBlock > 0n ? blocks.get(latestBlock) : undefined;
  const baseFee = latestBlockData
    ? (Number(latestBlockData.baseFeePerGas) / 1e9).toFixed(2) + ' gwei'
    : '--';

  return (
    <div>
      <PanelHeader icon="stats" title="Chain Stats" />
      <div
        style={{
          display: 'grid',
          gridTemplateColumns: '1fr 1fr',
          gap: '1px',
          background: 'rgba(255,255,255,0.04)',
        }}
      >
        {/* Cell 1: Block Height */}
        <div style={{ background: 'rgba(12, 12, 16, 0.9)', padding: '10px 14px' }}>
          <div
            style={{
              fontFamily: 'var(--rd-font-mono)',
              fontSize: '11px',
              textTransform: 'uppercase',
              letterSpacing: '0.08em',
              color: '#8a7a82',
              marginBottom: 4,
            }}
          >
            Block Height
          </div>
          {latestBlock > 0n ? (
            <AnimatedNumber
              value={Number(latestBlock)}
              format={(n) => '#' + Math.round(n).toLocaleString()}
              style={{ fontFamily: 'var(--rd-font-mono)', fontSize: '14px', color: '#dca5bd' }}
            />
          ) : (
            <div style={{ fontFamily: 'var(--rd-font-mono)', fontSize: '14px', color: '#dca5bd' }}>
              --
            </div>
          )}
        </div>

        {/* Cell 2: Block Time Sparkline */}
        <div style={{ background: 'rgba(12, 12, 16, 0.9)', padding: '10px 14px' }}>
          <div
            style={{
              fontFamily: 'var(--rd-font-mono)',
              fontSize: '11px',
              textTransform: 'uppercase',
              letterSpacing: '0.08em',
              color: '#8a7a82',
              marginBottom: 4,
            }}
          >
            Block Time
          </div>
          <BlockTimeSparkline width={140} height={32} />
        </div>

        {/* Cell 3: Base Fee */}
        <div style={{ background: 'rgba(12, 12, 16, 0.9)' }}>
          <Stat label="Base Fee" value={baseFee} color="#e8a84c" />
        </div>

        {/* Cell 4: Avg Gas Usage */}
        <div style={{ background: 'rgba(12, 12, 16, 0.9)' }}>
          <Stat label="Avg Gas Usage" value={avgGas} />
        </div>

        {/* Cell 5: Recent TXs */}
        <div style={{ background: 'rgba(12, 12, 16, 0.9)', padding: '10px 14px' }}>
          <div
            style={{
              fontFamily: 'var(--rd-font-mono)',
              fontSize: '11px',
              textTransform: 'uppercase',
              letterSpacing: '0.08em',
              color: '#8a7a82',
              marginBottom: 4,
            }}
          >
            Recent TXs
          </div>
          <AnimatedNumber
            value={totalTxs}
            style={{ fontFamily: 'var(--rd-font-mono)', fontSize: '14px', color: '#d8c8c4' }}
          />
        </div>

        {/* Cell 6: Gas Used Chart */}
        <div style={{ background: 'rgba(12, 12, 16, 0.9)', padding: '10px 14px' }}>
          <div
            style={{
              fontFamily: 'var(--rd-font-mono)',
              fontSize: '11px',
              textTransform: 'uppercase',
              letterSpacing: '0.08em',
              color: '#8a7a82',
              marginBottom: 4,
            }}
          >
            Gas Used
          </div>
          <GasUsageChart width={140} height={32} />
        </div>
      </div>

      {/* Base Fee Trend */}
      <div>
        <div style={chartLabelStyle}>Base Fee Trend</div>
        <FeeTrendChart />
      </div>

      {/* Tx Volume */}
      <div>
        <div style={chartLabelStyle}>Tx Volume</div>
        <TxVolumeChart />
      </div>
    </div>
  );
}
