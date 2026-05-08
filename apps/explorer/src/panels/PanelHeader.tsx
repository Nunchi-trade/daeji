import React from 'react';

interface PanelHeaderProps {
  icon: 'blocks' | 'consensus' | 'stats' | 'txs';
  title: string;
  badge?: React.ReactNode;
}

const icons: Record<PanelHeaderProps['icon'], React.ReactNode> = {
  blocks: (
    <svg width="14" height="14" viewBox="0 0 14 14" fill="none" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" strokeLinejoin="round">
      <path d="M7 1L12 4V10L7 13L2 10V4L7 1Z" />
      <path d="M7 1V7M7 7L12 4M7 7L2 4" />
    </svg>
  ),
  consensus: (
    <svg width="14" height="14" viewBox="0 0 14 14" fill="none" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round">
      <circle cx="7" cy="7" r="5" />
      <circle cx="7" cy="7" r="2" />
    </svg>
  ),
  stats: (
    <svg width="14" height="14" viewBox="0 0 14 14" fill="none" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" strokeLinejoin="round">
      <line x1="2" y1="12" x2="2" y2="8" />
      <line x1="7" y1="12" x2="7" y2="4" />
      <line x1="12" y1="12" x2="12" y2="6" />
    </svg>
  ),
  txs: (
    <svg width="14" height="14" viewBox="0 0 14 14" fill="none" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" strokeLinejoin="round">
      <line x1="2" y1="7" x2="12" y2="7" />
      <polyline points="8,3 12,7 8,11" />
    </svg>
  ),
};

export function PanelHeader({ icon, title, badge }: PanelHeaderProps) {
  return (
    <div style={{
      position: 'relative',
      padding: '10px 14px',
      fontFamily: 'var(--rd-font-mono)',
      fontSize: '12px',
      fontWeight: 600,
      textTransform: 'uppercase',
      letterSpacing: '0.1em',
      color: '#aa7088',
      borderBottom: '1px solid rgba(255,255,255,0.06)',
      display: 'flex',
      flexDirection: 'row',
      alignItems: 'center',
      justifyContent: 'space-between',
      gap: '8px',
    }}>
      <div style={{ display: 'flex', flexDirection: 'row', alignItems: 'center', gap: '8px' }}>
        <span style={{ color: 'var(--rd-rose-dim, #6a4058)', display: 'flex', alignItems: 'center' }}>
          {icons[icon]}
        </span>
        {title}
      </div>
      {badge != null && <div>{badge}</div>}
      <div style={{
        position: 'absolute',
        bottom: 0,
        left: 0,
        right: 0,
        height: '2px',
        background: 'linear-gradient(to right, var(--rd-rose-dim, #6a4058), transparent)',
      }} />
    </div>
  );
}
