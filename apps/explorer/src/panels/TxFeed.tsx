import { AnimatePresence, motion } from 'framer-motion';
import { useChainStore } from '@/data/store';
import { truncateAddress, formatEth } from '@/lib/format';
import { PanelHeader } from '@/panels/PanelHeader';

const METHOD_NAMES: Record<string, string> = {
  '0xa9059cbb': 'transfer',
  '0x23b872dd': 'transferFrom',
  '0x095ea7b3': 'approve',
  '0x40c10f19': 'mint',
  '0x42966c68': 'burn',
  '0x3593564c': 'execute',
  '0x38ed1739': 'swap',
  '0x5ae401dc': 'multicall',
};

function getMethodName(input: string): string | null {
  if (!input || input === '0x' || input.length < 10) return null;
  const selector = input.slice(0, 10).toLowerCase();
  return METHOD_NAMES[selector] ?? null;
}

function getDotColor(value: bigint): { color: string; glow?: boolean } {
  if (value === 0n) return { color: 'var(--rd-text-ghost, #3a303a)' };
  const eth = Number(value) / 1e18;
  if (eth < 0.01) return { color: 'var(--rd-dream, #7a7a98)' };
  if (eth < 1) return { color: 'var(--rd-bone, #b8a880)' };
  return { color: 'var(--rd-bone-bright, #d8c8a0)', glow: true };
}

export function TxFeed() {
  const blockOrder = useChainStore((s) => s.blockOrder);
  const blocks = useChainStore((s) => s.blocks);

  const recentTxs: Array<{
    hash: string;
    from: string;
    to: string | null;
    value: bigint;
    blockNumber: bigint;
    input: string;
  }> = [];

  for (const num of blockOrder.slice(0, 50)) {
    const b = blocks.get(num);
    if (!b) continue;
    for (const tx of b.transactions) {
      recentTxs.push({
        hash: tx.hash,
        from: tx.from,
        to: tx.to,
        value: tx.value,
        blockNumber: b.number,
        input: tx.input,
      });
      if (recentTxs.length >= 25) break;
    }
    if (recentTxs.length >= 25) break;
  }

  return (
    <div>
      <PanelHeader icon="txs" title="Recent Transactions" />
      <div
        style={{
          overflowY: 'auto',
          maxHeight: 280,
          scrollbarWidth: 'thin',
          scrollbarColor: 'rgba(170,112,136,0.3) transparent',
        }}
      >
        {recentTxs.length === 0 && (
          <div
            style={{
              padding: '24px 14px',
              fontFamily: 'var(--rd-font-mono)',
              fontSize: '13px',
              color: '#6a5a62',
              textAlign: 'center',
              lineHeight: 1.6,
            }}
          >
            No transactions yet
            <br />
            <span style={{ fontSize: '11px', color: '#4a4048' }}>
              Chain is producing empty blocks
            </span>
          </div>
        )}
        <AnimatePresence initial={false}>
          {recentTxs.map((tx) => {
            const dot = getDotColor(tx.value);
            const method = getMethodName(tx.input);
            return (
              <motion.div
                key={tx.hash}
                initial={{ opacity: 0, x: 20 }}
                animate={{ opacity: 1, x: 0 }}
                exit={{ opacity: 0, x: 20 }}
                transition={{ duration: 0.3, ease: [0.16, 1, 0.3, 1] }}
                style={{
                  display: 'flex',
                  flexDirection: 'column',
                  gap: 4,
                  padding: '8px 14px',
                  borderBottom: '1px solid rgba(255,255,255,0.04)',
                  fontFamily: 'var(--rd-font-mono)',
                  fontSize: '13px',
                  cursor: 'pointer',
                  transition: 'background 80ms ease-out',
                }}
                onMouseEnter={(e) => {
                  (e.currentTarget as HTMLElement).style.background = 'rgba(255,255,255,0.03)';
                }}
                onMouseLeave={(e) => {
                  (e.currentTarget as HTMLElement).style.background = 'transparent';
                }}
              >
                <div
                  style={{
                    display: 'flex',
                    alignItems: 'center',
                    gap: 8,
                  }}
                >
                  <span
                    style={{
                      width: 6,
                      height: 6,
                      borderRadius: '50%',
                      flexShrink: 0,
                      background: dot.color,
                      boxShadow: dot.glow ? `0 0 6px ${dot.color}` : undefined,
                    }}
                  />
                  <span style={{ color: '#7a7a98' }}>
                    {truncateAddress(tx.from)}
                  </span>
                  <div style={{ display: 'flex', alignItems: 'center', gap: 4, flexShrink: 0 }}>
                    {method && (
                      <span
                        style={{
                          fontSize: '9px',
                          color: 'var(--rd-text-dim)',
                          background: 'rgba(255,255,255,0.04)',
                          padding: '1px 4px',
                          letterSpacing: '0.04em',
                        }}
                      >
                        {method}
                      </span>
                    )}
                    <svg width="16" height="10" viewBox="0 0 16 10" style={{ flexShrink: 0 }}>
                      <line x1="0" y1="5" x2="12" y2="5" stroke="var(--rd-text-ghost)" strokeWidth="1" />
                      <polyline points="10,2 13,5 10,8" fill="none" stroke="var(--rd-text-ghost)" strokeWidth="1" />
                    </svg>
                  </div>
                  <span style={{ color: tx.to ? '#d8c8a0' : '#dca5bd' }}>
                    {tx.to ? truncateAddress(tx.to) : 'CONTRACT CREATE'}
                  </span>
                </div>
                {tx.value > 0n && (
                  <span style={{ color: '#8a7a82', fontSize: '12px', paddingLeft: 14 }}>
                    {formatEth(tx.value)} ETH
                  </span>
                )}
              </motion.div>
            );
          })}
        </AnimatePresence>
      </div>
    </div>
  );
}
