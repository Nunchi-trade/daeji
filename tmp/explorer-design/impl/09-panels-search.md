# 09 -- Panels, Search & Detail Views

Overlay panels, search, and detail views that sit on top of the WebGL scenes.
All components are React, styled with ROSEDUST glass panel CSS, positioned with CSS
(`position: fixed` / `position: absolute`), and never rendered inside WebGL space.

Reference specs:
- [`../05-interaction.md`](../05-interaction.md) -- interaction model, search, detail views
- [`../02-visual-architecture.md`](../02-visual-architecture.md) -- glass panel system, typography mapping, layout modes
- [`../04-data-flows.md`](../04-data-flows.md) -- Zustand stores, EventBus, RPC client, type definitions

Depends on:
- `02-rosedust-tokens.md` (CSS custom properties, glass.module.css)
- `04-state-stores.md` (Zustand chain/ui/consensus stores, EventBus, types)

---

## Architecture

```
   Scenes (WebGL / Canvas)           Panels (React DOM)
  +--------------------------+      +----------------------------+
  |  TerrainScene            |      |  GlassPanel (base)         |
  |  ConstellationCanvas     |      |  StatusBar                 |
  |  WaterfallScene          |      |  SceneNav                  |
  |  ConsensusRing           |      |  BlockDetail               |
  |                          |      |  TxDetail                  |
  | z-index: var(--rd-z-scene) |    |  AddressDetail             |
  +--------------------------+      |  SearchOverlay             |
                                    |  Tooltip                   |
                                    |  z-index: var(--rd-z-panels)|
                                    +----------------------------+
```

Key principles:

1. **React components with ROSEDUST glass panel styling.** Every panel inherits from the
   `GlassPanel` base component or uses `glass.module.css` classes directly.
2. **Positioned with CSS, not in WebGL space.** Panels are `position: fixed` overlays
   on top of the scene canvas. The scene canvas is `z-index: var(--rd-z-scene)` (1).
   Panels are `z-index: var(--rd-z-panels)` (100) and higher.
3. **Read from Zustand stores.** Panels subscribe to `useChainStore` and `useUIStore`
   via selectors. Only re-render when their specific slice changes.
4. **Emit events to bus for scene interaction.** When a panel action should affect a
   scene (e.g., "highlight this block in the terrain"), the panel emits an event on
   the mitt EventBus. Scenes listen imperatively -- no React re-renders on the scene side.
5. **All panels are optional.** Scenes work without them. Panels enhance the experience
   but the WebGL scenes are self-contained. If panels fail to render, the scenes continue.

---

## 9.1 Glass Panel Base Component

The foundational wrapper used by every panel in the explorer. Provides consistent glass
styling, optional collapse/expand behavior, close button, and positioning.

### 9.1.1 `apps/explorer/src/panels/GlassPanel.tsx`

```tsx
import { type ReactNode, useState, useCallback, useEffect, useRef } from 'react';
import { motion, AnimatePresence } from 'framer-motion';
import styles from '@/design/glass.module.css';

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

interface GlassPanelProps {
  children: ReactNode;
  position: 'left' | 'right' | 'bottom' | 'center';
  width?: string;
  collapsible?: boolean;
  title?: string;
  active?: boolean;
  onClose?: () => void;
  className?: string;
  /** Allow drag-to-resize on the right panel edge. Only meaningful for left/right. */
  resizable?: boolean;
  /** aria-label for the panel region */
  ariaLabel?: string;
  /** Test id for testing-library queries */
  testId?: string;
}

// ---------------------------------------------------------------------------
// Position CSS map
// ---------------------------------------------------------------------------

const positionStyles: Record<GlassPanelProps['position'], React.CSSProperties> = {
  left: {
    position: 'fixed',
    top: 'var(--rd-space-lg)',
    left: 'var(--rd-space-lg)',
    bottom: 'var(--rd-space-lg)',
    zIndex: 'var(--rd-z-panels)' as unknown as number,
  },
  right: {
    position: 'fixed',
    top: 'var(--rd-space-lg)',
    right: 'var(--rd-space-lg)',
    bottom: 'var(--rd-space-lg)',
    zIndex: 'var(--rd-z-panels)' as unknown as number,
  },
  bottom: {
    position: 'fixed',
    left: 0,
    right: 0,
    bottom: 0,
    zIndex: 'var(--rd-z-panels)' as unknown as number,
  },
  center: {
    position: 'fixed',
    top: '50%',
    left: '50%',
    transform: 'translate(-50%, -50%)',
    zIndex: 'var(--rd-z-detail)' as unknown as number,
  },
};

// ---------------------------------------------------------------------------
// Framer Motion variants
// ---------------------------------------------------------------------------

const panelVariants = {
  left: {
    initial: { x: -40, opacity: 0 },
    animate: { x: 0, opacity: 1 },
    exit: { x: -40, opacity: 0 },
  },
  right: {
    initial: { x: 40, opacity: 0 },
    animate: { x: 0, opacity: 1 },
    exit: { x: 40, opacity: 0 },
  },
  bottom: {
    initial: { y: 20, opacity: 0 },
    animate: { y: 0, opacity: 1 },
    exit: { y: 20, opacity: 0 },
  },
  center: {
    initial: { scale: 0.96, opacity: 0 },
    animate: { scale: 1, opacity: 1 },
    exit: { scale: 0.96, opacity: 0 },
  },
};

const transitionConfig = {
  duration: 0.3,
  ease: [0.16, 1, 0.3, 1], // --rd-ease-out
};

// ---------------------------------------------------------------------------
// Component
// ---------------------------------------------------------------------------

export function GlassPanel({
  children,
  position,
  width,
  collapsible = false,
  title,
  active = false,
  onClose,
  className,
  resizable = false,
  ariaLabel,
  testId,
}: GlassPanelProps) {
  const [collapsed, setCollapsed] = useState(false);
  const [panelWidth, setPanelWidth] = useState<number | null>(null);
  const panelRef = useRef<HTMLDivElement>(null);
  const dragRef = useRef<{ startX: number; startW: number } | null>(null);

  // ── Keyboard: Escape to close ──
  useEffect(() => {
    if (!onClose) return;
    const handler = (e: KeyboardEvent) => {
      if (e.key === 'Escape') {
        e.stopPropagation();
        onClose();
      }
    };
    window.addEventListener('keydown', handler);
    return () => window.removeEventListener('keydown', handler);
  }, [onClose]);

  // ── Drag to resize (right panel width) ──
  const onResizeStart = useCallback(
    (e: React.MouseEvent) => {
      if (!resizable || !panelRef.current) return;
      e.preventDefault();
      const rect = panelRef.current.getBoundingClientRect();
      dragRef.current = { startX: e.clientX, startW: rect.width };

      const onMove = (me: MouseEvent) => {
        if (!dragRef.current) return;
        const delta =
          position === 'right'
            ? dragRef.current.startX - me.clientX
            : me.clientX - dragRef.current.startX;
        const newWidth = Math.max(280, Math.min(800, dragRef.current.startW + delta));
        setPanelWidth(newWidth);
      };

      const onUp = () => {
        dragRef.current = null;
        document.removeEventListener('mousemove', onMove);
        document.removeEventListener('mouseup', onUp);
      };

      document.addEventListener('mousemove', onMove);
      document.addEventListener('mouseup', onUp);
    },
    [resizable, position],
  );

  // ── Resolved width ──
  const resolvedWidth = panelWidth ? `${panelWidth}px` : width;

  // ── Panel CSS class list ──
  const panelClass = [
    styles.panel,
    active ? styles.panelActive : '',
    className ?? '',
  ]
    .filter(Boolean)
    .join(' ');

  const variants = panelVariants[position];

  return (
    <AnimatePresence>
      <motion.div
        ref={panelRef}
        className={panelClass}
        style={{
          ...positionStyles[position],
          width: resolvedWidth,
          overflow: 'hidden',
          pointerEvents: 'auto',
        }}
        role="region"
        aria-label={ariaLabel ?? title ?? 'Panel'}
        data-testid={testId}
        initial={variants.initial}
        animate={variants.animate}
        exit={variants.exit}
        transition={transitionConfig}
      >
        {/* ── Header ── */}
        {(title || collapsible || onClose) && (
          <div
            style={{
              display: 'flex',
              alignItems: 'center',
              justifyContent: 'space-between',
              padding: 'var(--rd-space-sm) var(--rd-space-md)',
              borderBottom: '1px solid var(--rd-border)',
              minHeight: 32,
            }}
          >
            {/* Title label */}
            {title && <span className={styles.label}>{title}</span>}

            {/* Spacer */}
            <span style={{ flex: 1 }} />

            {/* Collapse toggle */}
            {collapsible && (
              <button
                onClick={() => setCollapsed((c) => !c)}
                aria-label={collapsed ? 'Expand panel' : 'Collapse panel'}
                style={{
                  background: 'none',
                  border: 'none',
                  color: 'var(--rd-text-dim)',
                  cursor: 'pointer',
                  fontFamily: 'var(--rd-font-mono)',
                  fontSize: 'var(--rd-text-sm)',
                  padding: 'var(--rd-space-xs)',
                  letterSpacing: 'var(--rd-tracking-normal)',
                }}
              >
                {collapsed ? '+' : '\u2013'}
              </button>
            )}

            {/* Close button */}
            {onClose && (
              <button
                onClick={onClose}
                aria-label="Close panel"
                style={{
                  background: 'none',
                  border: 'none',
                  color: 'var(--rd-text-dim)',
                  cursor: 'pointer',
                  fontFamily: 'var(--rd-font-mono)',
                  fontSize: 'var(--rd-text-base)',
                  padding: 'var(--rd-space-xs)',
                  marginLeft: 'var(--rd-space-xs)',
                  lineHeight: 1,
                }}
              >
                &times;
              </button>
            )}
          </div>
        )}

        {/* ── Body (collapsible) ── */}
        <AnimatePresence initial={false}>
          {!collapsed && (
            <motion.div
              key="panel-body"
              initial={{ height: 0, opacity: 0 }}
              animate={{ height: 'auto', opacity: 1 }}
              exit={{ height: 0, opacity: 0 }}
              transition={{ duration: 0.2, ease: [0.16, 1, 0.3, 1] }}
              style={{ overflow: 'hidden' }}
            >
              {children}
            </motion.div>
          )}
        </AnimatePresence>

        {/* ── Resize handle (drag) ── */}
        {resizable && (position === 'left' || position === 'right') && (
          <div
            onMouseDown={onResizeStart}
            style={{
              position: 'absolute',
              top: 0,
              bottom: 0,
              width: 6,
              cursor: 'col-resize',
              ...(position === 'right' ? { left: -3 } : { right: -3 }),
            }}
          />
        )}
      </motion.div>
    </AnimatePresence>
  );
}
```

### 9.1.2 Glass panel CSS additions

The base `glass.module.css` from doc 02 already contains `.panel`, `.panelActive`,
`.label`, and `.led`. The following additional classes support the panels in this doc.
Append to `apps/explorer/src/design/glass.module.css`:

```css
/* ── Panel content padding ── */
.panelBody {
  padding: var(--rd-space-md);
}

/* ── Panel header with title + controls ── */
.panelHeader {
  display: flex;
  align-items: center;
  gap: var(--rd-space-sm);
  padding: var(--rd-space-sm) var(--rd-space-md);
  border-bottom: 1px solid var(--rd-border);
  min-height: 32px;
}

/* ── Copy button (inline, appears on hash hover) ── */
.copyBtn {
  background: none;
  border: none;
  color: var(--rd-text-dim);
  cursor: pointer;
  font-family: var(--rd-font-mono);
  font-size: var(--rd-text-xs);
  padding: 2px 4px;
  letter-spacing: var(--rd-tracking-normal);
  opacity: 0;
  transition: opacity var(--rd-duration-fast) var(--rd-ease-out);
}

.copyBtn:hover {
  color: var(--rd-bone-bright);
}

/* Show copy button on parent hover */
.hashRow:hover .copyBtn {
  opacity: 1;
}

/* ── Hash row (hash value + copy action) ── */
.hashRow {
  display: flex;
  align-items: center;
  gap: var(--rd-space-xs);
  font-family: var(--rd-font-mono);
  font-size: var(--rd-text-sm);
  color: var(--rd-text-dim);
  letter-spacing: var(--rd-tracking-normal);
  cursor: pointer;
}

/* Flash feedback on copy */
.hashRow.copied {
  color: var(--rd-bone-bright);
  transition: color 0ms;
}

/* ── Stats grid (2-column metrics) ── */
.statsGrid {
  display: grid;
  grid-template-columns: 1fr 1fr;
  gap: 1px;
  background: var(--rd-border);
}

.statCell {
  background: var(--rd-glass-bg);
  padding: var(--rd-space-sm) var(--rd-space-md);
}

.statLabel {
  font-family: var(--rd-font-mono);
  font-size: var(--rd-text-xs);
  text-transform: uppercase;
  letter-spacing: var(--rd-tracking-label);
  color: var(--rd-text-dim);
  margin-bottom: var(--rd-space-xs);
}

.statValue {
  font-family: var(--rd-font-mono);
  font-size: var(--rd-text-base);
  color: var(--rd-text);
}

/* ── Gas bar (inline usage bar) ── */
.gasBar {
  height: 4px;
  background: var(--rd-void-light);
  position: relative;
  margin-top: var(--rd-space-xs);
}

.gasBarFill {
  position: absolute;
  top: 0;
  left: 0;
  bottom: 0;
  background: var(--rd-warning);
  transition: width var(--rd-duration-normal) var(--rd-ease-out);
}

/* ── Scrollable list ── */
.scrollList {
  max-height: 300px;
  overflow-y: auto;
  scrollbar-width: thin;
  scrollbar-color: var(--rd-text-ghost) transparent;
}

.scrollList::-webkit-scrollbar {
  width: 4px;
}

.scrollList::-webkit-scrollbar-track {
  background: transparent;
}

.scrollList::-webkit-scrollbar-thumb {
  background: var(--rd-text-ghost);
}

/* ── List row (clickable) ── */
.listRow {
  display: flex;
  align-items: center;
  gap: var(--rd-space-sm);
  padding: var(--rd-space-sm) var(--rd-space-md);
  border-bottom: 1px solid var(--rd-border);
  cursor: pointer;
  transition:
    background var(--rd-duration-fast) var(--rd-ease-out),
    transform var(--rd-duration-fast) var(--rd-ease-out);
}

.listRow:hover {
  background: var(--rd-glass-bg-hover);
  transform: translateY(-1px);
}

.listRow:active {
  transform: translateY(0);
}

/* ── Status badge (success / reverted) ── */
.badge {
  display: inline-flex;
  align-items: center;
  gap: var(--rd-space-xs);
  font-family: var(--rd-font-mono);
  font-size: var(--rd-text-xs);
  text-transform: uppercase;
  letter-spacing: var(--rd-tracking-label);
  padding: 2px 6px;
  border: 1px solid var(--rd-border);
}

.badgeSuccess {
  color: var(--rd-success);
  border-color: var(--rd-success);
}

.badgeReverted {
  color: var(--rd-danger);
  border-color: var(--rd-danger);
}

.badgeContract {
  color: var(--rd-rose);
  border-color: var(--rd-border-rose);
}

.badgeEoa {
  color: var(--rd-dream);
  border-color: var(--rd-dream-dim);
}

/* ── Navigation arrows ── */
.navArrow {
  background: none;
  border: 1px solid var(--rd-border);
  color: var(--rd-text-dim);
  cursor: pointer;
  font-family: var(--rd-font-mono);
  font-size: var(--rd-text-base);
  padding: var(--rd-space-xs) var(--rd-space-sm);
  transition:
    color var(--rd-duration-fast) var(--rd-ease-out),
    border-color var(--rd-duration-fast) var(--rd-ease-out);
}

.navArrow:hover {
  color: var(--rd-text);
  border-color: var(--rd-border-strong);
}

.navArrow:disabled {
  opacity: 0.3;
  cursor: default;
}
```

### 9.1.3 Verification

- [ ] GlassPanel renders with correct glass background, border, and backdrop blur
- [ ] Title label shows in uppercase with tracking
- [ ] Close button fires `onClose` callback
- [ ] Escape key fires `onClose` callback
- [ ] Collapse/expand animates body height with framer-motion
- [ ] Drag to resize changes panel width (min 280px, max 800px)
- [ ] `position="left"` anchors to left edge, `position="right"` to right edge
- [ ] Active panel has left rose border accent
- [ ] Panel has `role="region"` with `aria-label`

---

## 9.2 Block Detail Panel

Shown when a block is selected. Full block data with hash art, transaction list,
and navigation arrows.

### 9.2.1 `apps/explorer/src/panels/BlockDetail.tsx`

```tsx
import { useEffect, useState, useCallback, useRef } from 'react';
import { useNavigate, useParams } from 'react-router';
import { GlassPanel } from './GlassPanel';
import { HashArt } from './HashArt';
import { useChainStore } from '@/data/store';
import { httpClient } from '@/data/rpc';
import { bus } from '@/data/bus';
import {
  truncateHash,
  formatEth,
  formatGas,
  formatTimestamp,
  formatNumber,
} from '@/utils/format';
import type { ChainBlock, ChainTransaction } from '@/data/types';
import styles from '@/design/glass.module.css';

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

interface BlockDetailData {
  block: ChainBlock;
  loading: boolean;
  error: string | null;
}

// ---------------------------------------------------------------------------
// Data fetching hook
// ---------------------------------------------------------------------------

function useBlockData(numberOrHash: string | undefined): BlockDetailData {
  const cachedBlocks = useChainStore((s) => s.blocks);
  const [block, setBlock] = useState<ChainBlock | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    if (!numberOrHash) return;

    let cancelled = false;

    async function fetch() {
      setLoading(true);
      setError(null);

      try {
        // Try cache first
        const isNumber = /^\d+$/.test(numberOrHash);
        const isHex = /^0x[0-9a-fA-F]+$/.test(numberOrHash);

        if (isNumber) {
          const num = BigInt(numberOrHash);
          const cached = cachedBlocks.get(num);
          if (cached) {
            if (!cancelled) {
              setBlock(cached);
              setLoading(false);
            }
            return;
          }
        }

        // Fetch from RPC
        const param = isNumber ? `0x${BigInt(numberOrHash).toString(16)}` : numberOrHash;
        const result = await httpClient.request({
          method: 'eth_getBlockByNumber',
          params: [param as `0x${string}`, true],
        });

        if (!cancelled && result) {
          setBlock(result as unknown as ChainBlock);
        } else if (!cancelled) {
          setError('Block not found');
        }
      } catch (err) {
        if (!cancelled) {
          setError(err instanceof Error ? err.message : 'Failed to fetch block');
        }
      } finally {
        if (!cancelled) setLoading(false);
      }
    }

    fetch();
    return () => {
      cancelled = true;
    };
  }, [numberOrHash, cachedBlocks]);

  return { block: block!, loading, error };
}

// ---------------------------------------------------------------------------
// Copy to clipboard helper
// ---------------------------------------------------------------------------

function useCopyToClipboard() {
  const [copiedField, setCopiedField] = useState<string | null>(null);
  const timerRef = useRef<ReturnType<typeof setTimeout>>();

  const copy = useCallback((text: string, field: string) => {
    navigator.clipboard.writeText(text);
    setCopiedField(field);
    clearTimeout(timerRef.current);
    timerRef.current = setTimeout(() => setCopiedField(null), 1500);
  }, []);

  return { copiedField, copy };
}

// ---------------------------------------------------------------------------
// Sub-components
// ---------------------------------------------------------------------------

function HashField({
  label,
  hash,
  copiedField,
  onCopy,
}: {
  label: string;
  hash: `0x${string}`;
  copiedField: string | null;
  onCopy: (text: string, field: string) => void;
}) {
  const isCopied = copiedField === label;

  return (
    <div style={{ marginBottom: 'var(--rd-space-sm)' }}>
      <div className={styles.statLabel}>{label}</div>
      <div
        className={`${styles.hashRow} ${isCopied ? styles.copied : ''}`}
        onClick={() => onCopy(hash, label)}
        title={hash}
        role="button"
        tabIndex={0}
        onKeyDown={(e) => {
          if (e.key === 'Enter' || e.key === ' ') onCopy(hash, label);
        }}
      >
        <code>{truncateHash(hash, 8)}</code>
        <span className={styles.copyBtn}>{isCopied ? 'copied' : 'copy'}</span>
      </div>
    </div>
  );
}

function GasBar({ used, limit }: { used: bigint; limit: bigint }) {
  const pct = limit > 0n ? Number((used * 100n) / limit) : 0;

  return (
    <div>
      <div
        style={{
          display: 'flex',
          justifyContent: 'space-between',
          fontFamily: 'var(--rd-font-mono)',
          fontSize: 'var(--rd-text-sm)',
        }}
      >
        <span style={{ color: 'var(--rd-text)' }}>{formatGas(used)}</span>
        <span style={{ color: 'var(--rd-text-dim)' }}>/ {formatGas(limit)}</span>
      </div>
      <div className={styles.gasBar}>
        <div className={styles.gasBarFill} style={{ width: `${pct}%` }} />
      </div>
    </div>
  );
}

function TransactionRow({
  tx,
  onClick,
}: {
  tx: ChainTransaction;
  onClick: (hash: `0x${string}`) => void;
}) {
  return (
    <div
      className={styles.listRow}
      onClick={() => onClick(tx.hash)}
      role="button"
      tabIndex={0}
      onKeyDown={(e) => {
        if (e.key === 'Enter') onClick(tx.hash);
      }}
    >
      <code
        style={{
          fontFamily: 'var(--rd-font-mono)',
          fontSize: 'var(--rd-text-sm)',
          color: 'var(--rd-bone-bright)',
          flex: '0 0 120px',
        }}
      >
        {truncateHash(tx.hash, 4)}
      </code>
      <span
        style={{
          fontFamily: 'var(--rd-font-mono)',
          fontSize: 'var(--rd-text-xs)',
          color: 'var(--rd-text-dim)',
          flex: 1,
        }}
      >
        {truncateHash(tx.from, 4)} &rarr; {tx.to ? truncateHash(tx.to, 4) : 'CREATE'}
      </span>
      <span
        style={{
          fontFamily: 'var(--rd-font-mono)',
          fontSize: 'var(--rd-text-sm)',
          color: 'var(--rd-bone)',
          flex: '0 0 100px',
          textAlign: 'right',
        }}
      >
        {formatEth(tx.value)} ETH
      </span>
    </div>
  );
}

// ---------------------------------------------------------------------------
// Main Component
// ---------------------------------------------------------------------------

export function BlockDetail() {
  const { numberOrHash } = useParams<{ numberOrHash: string }>();
  const navigate = useNavigate();
  const { block, loading, error } = useBlockData(numberOrHash);
  const { copiedField, copy } = useCopyToClipboard();

  // ── Navigation: prev/next block arrows ──
  const goToBlock = useCallback(
    (num: bigint) => {
      navigate(`/block/${num}`);
    },
    [navigate],
  );

  const goToTx = useCallback(
    (hash: `0x${string}`) => {
      navigate(`/tx/${hash}`);
      bus.emit('tx:confirmed', { hash } as ChainTransaction);
    },
    [navigate],
  );

  const handleClose = useCallback(() => {
    navigate(-1);
  }, [navigate]);

  // ── Keyboard: left/right arrows for block navigation ──
  useEffect(() => {
    if (!block) return;
    const handler = (e: KeyboardEvent) => {
      if (e.key === 'ArrowLeft' && block.number > 0n) {
        goToBlock(block.number - 1n);
      } else if (e.key === 'ArrowRight') {
        goToBlock(block.number + 1n);
      }
    };
    window.addEventListener('keydown', handler);
    return () => window.removeEventListener('keydown', handler);
  }, [block, goToBlock]);

  // ── Loading state ──
  if (loading) {
    return (
      <GlassPanel position="right" width="480px" title="BLOCK" onClose={handleClose}>
        <div className={styles.panelBody}>
          <span style={{ color: 'var(--rd-text-dim)' }}>Loading...</span>
        </div>
      </GlassPanel>
    );
  }

  // ── Error state ──
  if (error || !block) {
    return (
      <GlassPanel position="right" width="480px" title="BLOCK" onClose={handleClose}>
        <div className={styles.panelBody}>
          <span style={{ color: 'var(--rd-danger)' }}>{error ?? 'Block not found'}</span>
        </div>
      </GlassPanel>
    );
  }

  const txCount = block.transactions?.length ?? 0;

  return (
    <GlassPanel
      position="right"
      width="480px"
      title={`BLOCK ${formatNumber(block.number)}`}
      onClose={handleClose}
      active
      resizable
      ariaLabel={`Block ${block.number} detail`}
      testId="block-detail"
    >
      <div className={styles.panelBody}>
        {/* ── Header: block number (hero) + timestamp ── */}
        <div
          style={{
            display: 'flex',
            justifyContent: 'space-between',
            alignItems: 'baseline',
            marginBottom: 'var(--rd-space-md)',
          }}
        >
          <span
            style={{
              fontFamily: 'var(--rd-font-display)',
              fontStyle: 'italic',
              fontSize: 'var(--rd-text-hero)',
              color: 'var(--rd-bone-bright)',
              lineHeight: 1,
            }}
          >
            {formatNumber(block.number)}
          </span>
          <span
            style={{
              fontFamily: 'var(--rd-font-mono)',
              fontSize: 'var(--rd-text-xs)',
              color: 'var(--rd-text-dim)',
              letterSpacing: 'var(--rd-tracking-normal)',
            }}
          >
            {formatTimestamp(block.timestamp)}
          </span>
        </div>

        {/* ── Hash Art (visual from block hash) ── */}
        <div
          style={{
            display: 'flex',
            justifyContent: 'center',
            marginBottom: 'var(--rd-space-md)',
          }}
        >
          <HashArt hash={block.hash} size={128} />
        </div>

        {/* ── Hash fields (copyable) ── */}
        <HashField label="HASH" hash={block.hash} copiedField={copiedField} onCopy={copy} />
        <HashField
          label="PARENT"
          hash={block.parentHash}
          copiedField={copiedField}
          onCopy={copy}
        />
        <HashField
          label="STATE ROOT"
          hash={block.stateRoot}
          copiedField={copiedField}
          onCopy={copy}
        />
        <HashField
          label="TRANSACTIONS ROOT"
          hash={block.transactionsRoot}
          copiedField={copiedField}
          onCopy={copy}
        />
        <HashField
          label="RECEIPTS ROOT"
          hash={block.receiptsRoot}
          copiedField={copiedField}
          onCopy={copy}
        />

        {/* ── Stats grid ── */}
        <div className={styles.statsGrid} style={{ marginTop: 'var(--rd-space-md)' }}>
          <div className={styles.statCell}>
            <div className={styles.statLabel}>GAS USED / LIMIT</div>
            <GasBar used={block.gasUsed} limit={block.gasLimit} />
          </div>
          <div className={styles.statCell}>
            <div className={styles.statLabel}>TRANSACTIONS</div>
            <div className={styles.statValue}>{txCount}</div>
          </div>
          <div className={styles.statCell}>
            <div className={styles.statLabel}>BASE FEE</div>
            <div className={styles.statValue}>
              {formatGas(block.baseFeePerGas)} <span style={{ color: 'var(--rd-text-dim)' }}>gwei</span>
            </div>
          </div>
          <div className={styles.statCell}>
            <div className={styles.statLabel}>TIMESTAMP</div>
            <div className={styles.statValue}>{block.timestamp.toString()}</div>
          </div>
        </div>

        {/* ── Transaction list ── */}
        <div style={{ marginTop: 'var(--rd-space-lg)' }}>
          <div
            className={styles.label}
            style={{ marginBottom: 'var(--rd-space-sm)', letterSpacing: 'var(--rd-tracking-section)' }}
          >
            &mdash;&mdash; TRANSACTIONS
          </div>

          {txCount === 0 ? (
            <div
              style={{
                fontFamily: 'var(--rd-font-mono)',
                fontSize: 'var(--rd-text-sm)',
                color: 'var(--rd-text-ghost)',
                padding: 'var(--rd-space-md)',
                textAlign: 'center',
              }}
            >
              No transactions in this block
            </div>
          ) : (
            <div className={styles.scrollList}>
              {block.transactions.map((tx) => (
                <TransactionRow key={tx.hash} tx={tx} onClick={goToTx} />
              ))}
            </div>
          )}
        </div>

        {/* ── Navigation: prev/next ── */}
        <div
          style={{
            display: 'flex',
            justifyContent: 'space-between',
            marginTop: 'var(--rd-space-lg)',
            paddingTop: 'var(--rd-space-md)',
            borderTop: '1px solid var(--rd-border)',
          }}
        >
          <button
            className={styles.navArrow}
            onClick={() => goToBlock(block.number - 1n)}
            disabled={block.number <= 0n}
            aria-label="Previous block"
          >
            &larr; BLOCK {formatNumber(block.number - 1n)}
          </button>
          <button
            className={styles.navArrow}
            onClick={() => goToBlock(block.number + 1n)}
            aria-label="Next block"
          >
            BLOCK {formatNumber(block.number + 1n)} &rarr;
          </button>
        </div>
      </div>
    </GlassPanel>
  );
}
```

### 9.2.2 Verification

- [ ] Header shows block number (Fraunces italic hero), hash (truncated), timestamp (relative)
- [ ] Stats grid displays gas used/limit bar, tx count, base fee
- [ ] Hash art canvas renders (128x128) from block hash
- [ ] Transaction list is scrollable, each row shows hash/from/to/value
- [ ] Clicking a transaction navigates to TxDetail
- [ ] State root, transactions root, receipts root are copyable (click flashes bone)
- [ ] Prev/next arrows navigate blocks, arrow keys work
- [ ] Data comes from chain store cache first, then `eth_getBlockByNumber`
- [ ] Loading and error states render correctly

---

## 9.3 Transaction Detail Panel

Shown when a transaction is selected. Displays full transaction data plus receipt.

### 9.3.1 `apps/explorer/src/panels/TxDetail.tsx`

```tsx
import { useEffect, useState, useCallback } from 'react';
import { useNavigate, useParams } from 'react-router';
import { GlassPanel } from './GlassPanel';
import { useChainStore } from '@/data/store';
import { httpClient } from '@/data/rpc';
import {
  truncateHash,
  formatEth,
  formatGas,
  formatNumber,
} from '@/utils/format';
import type { ChainTransaction, ChainReceipt, ChainLog } from '@/data/types';
import styles from '@/design/glass.module.css';

// ---------------------------------------------------------------------------
// Data fetching
// ---------------------------------------------------------------------------

interface TxDetailData {
  tx: ChainTransaction | null;
  receipt: ChainReceipt | null;
  loading: boolean;
  error: string | null;
}

function useTxData(hash: string | undefined): TxDetailData {
  const [tx, setTx] = useState<ChainTransaction | null>(null);
  const [receipt, setReceipt] = useState<ChainReceipt | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    if (!hash) return;
    let cancelled = false;

    async function fetch() {
      setLoading(true);
      setError(null);

      try {
        // Fetch tx + receipt in parallel
        const [txResult, receiptResult] = await Promise.all([
          httpClient.request({
            method: 'eth_getTransactionByHash',
            params: [hash as `0x${string}`],
          }),
          httpClient.request({
            method: 'eth_getTransactionReceipt',
            params: [hash as `0x${string}`],
          }),
        ]);

        if (!cancelled) {
          if (txResult) {
            setTx(txResult as unknown as ChainTransaction);
          } else {
            setError('Transaction not found');
          }
          if (receiptResult) {
            setReceipt(receiptResult as unknown as ChainReceipt);
          }
        }
      } catch (err) {
        if (!cancelled) {
          setError(err instanceof Error ? err.message : 'Failed to fetch transaction');
        }
      } finally {
        if (!cancelled) setLoading(false);
      }
    }

    fetch();
    return () => {
      cancelled = true;
    };
  }, [hash]);

  return { tx, receipt, loading, error };
}

// ---------------------------------------------------------------------------
// Sub-components
// ---------------------------------------------------------------------------

/** The from -> to flow visualization */
function FlowDiagram({
  from,
  to,
  value,
  onAddressClick,
}: {
  from: `0x${string}`;
  to: `0x${string}` | null;
  value: bigint;
  onAddressClick: (addr: `0x${string}`) => void;
}) {
  return (
    <div
      style={{
        display: 'flex',
        alignItems: 'center',
        justifyContent: 'space-between',
        padding: 'var(--rd-space-md)',
        borderBottom: '1px solid var(--rd-border)',
        gap: 'var(--rd-space-sm)',
      }}
    >
      {/* Sender */}
      <div
        style={{ textAlign: 'center', cursor: 'pointer' }}
        onClick={() => onAddressClick(from)}
        role="button"
        tabIndex={0}
        onKeyDown={(e) => {
          if (e.key === 'Enter') onAddressClick(from);
        }}
      >
        <code
          style={{
            fontFamily: 'var(--rd-font-mono)',
            fontSize: 'var(--rd-text-sm)',
            color: 'var(--rd-bone-bright)',
          }}
        >
          {truncateHash(from, 4)}
        </code>
        <div className={styles.label}>SENDER</div>
      </div>

      {/* Arrow + value */}
      <div style={{ textAlign: 'center', flex: 1 }}>
        <div
          style={{
            fontFamily: 'var(--rd-font-mono)',
            fontSize: 'var(--rd-text-sm)',
            color: 'var(--rd-bone)',
            marginBottom: 'var(--rd-space-xs)',
          }}
        >
          {formatEth(value)} ETH
        </div>
        <div
          style={{
            height: 1,
            background: 'var(--rd-bone-dim)',
            position: 'relative',
          }}
        >
          {/* Arrowhead */}
          <div
            style={{
              position: 'absolute',
              right: -4,
              top: -3,
              width: 0,
              height: 0,
              borderLeft: '6px solid var(--rd-bone-dim)',
              borderTop: '3px solid transparent',
              borderBottom: '3px solid transparent',
            }}
          />
        </div>
      </div>

      {/* Receiver */}
      <div
        style={{ textAlign: 'center', cursor: to ? 'pointer' : 'default' }}
        onClick={() => to && onAddressClick(to)}
        role="button"
        tabIndex={to ? 0 : -1}
        onKeyDown={(e) => {
          if (e.key === 'Enter' && to) onAddressClick(to);
        }}
      >
        <code
          style={{
            fontFamily: 'var(--rd-font-mono)',
            fontSize: 'var(--rd-text-sm)',
            color: to ? 'var(--rd-bone-bright)' : 'var(--rd-rose-bright)',
          }}
        >
          {to ? truncateHash(to, 4) : 'CONTRACT CREATE'}
        </code>
        <div className={styles.label}>{to ? 'RECEIVER' : 'CREATE'}</div>
      </div>
    </div>
  );
}

/** Status badge: success or reverted */
function StatusBadge({ status }: { status: 'success' | 'reverted' }) {
  const isSuccess = status === 'success';
  const cls = isSuccess ? styles.badgeSuccess : styles.badgeReverted;

  return (
    <span className={`${styles.badge} ${cls}`}>
      <span
        style={{
          width: 5,
          height: 5,
          borderRadius: '50%',
          background: 'currentColor',
          display: 'inline-block',
        }}
      />
      {status.toUpperCase()}
    </span>
  );
}

/** Log entry (decoded or raw) */
function LogEntry({ log, index }: { log: ChainLog; index: number }) {
  return (
    <div
      style={{
        padding: 'var(--rd-space-sm) var(--rd-space-md)',
        borderBottom: '1px solid var(--rd-border)',
      }}
    >
      <div
        style={{
          display: 'flex',
          alignItems: 'center',
          gap: 'var(--rd-space-sm)',
          marginBottom: 'var(--rd-space-xs)',
        }}
      >
        <span className={styles.label}>LOG {index}</span>
        <code
          style={{
            fontFamily: 'var(--rd-font-mono)',
            fontSize: 'var(--rd-text-xs)',
            color: 'var(--rd-text-dim)',
          }}
        >
          {truncateHash(log.address, 4)}
        </code>
      </div>

      {/* Topics */}
      {log.topics.map((topic, i) => (
        <div
          key={i}
          style={{
            fontFamily: 'var(--rd-font-mono)',
            fontSize: 'var(--rd-text-xs)',
            color: i === 0 ? 'var(--rd-dream)' : 'var(--rd-text-dim)',
            marginLeft: 'var(--rd-space-md)',
            marginBottom: 2,
            wordBreak: 'break-all',
          }}
        >
          [{i}] {truncateHash(topic, 8)}
        </div>
      ))}

      {/* Data */}
      {log.data !== '0x' && (
        <div
          style={{
            fontFamily: 'var(--rd-font-mono)',
            fontSize: 'var(--rd-text-xs)',
            color: 'var(--rd-text-ghost)',
            marginLeft: 'var(--rd-space-md)',
            marginTop: 'var(--rd-space-xs)',
            wordBreak: 'break-all',
            maxHeight: 60,
            overflow: 'hidden',
          }}
        >
          data: {truncateHash(log.data, 16)}
        </div>
      )}
    </div>
  );
}

// ---------------------------------------------------------------------------
// Main Component
// ---------------------------------------------------------------------------

export function TxDetail() {
  const { hash } = useParams<{ hash: string }>();
  const navigate = useNavigate();
  const { tx, receipt, loading, error } = useTxData(hash);

  const handleClose = useCallback(() => navigate(-1), [navigate]);

  const goToAddress = useCallback(
    (addr: `0x${string}`) => navigate(`/address/${addr}`),
    [navigate],
  );

  const goToBlock = useCallback(
    (num: bigint) => navigate(`/block/${num}`),
    [navigate],
  );

  // ── Loading ──
  if (loading) {
    return (
      <GlassPanel position="right" width="520px" title="TRANSACTION" onClose={handleClose}>
        <div className={styles.panelBody}>
          <span style={{ color: 'var(--rd-text-dim)' }}>Loading...</span>
        </div>
      </GlassPanel>
    );
  }

  // ── Error ──
  if (error || !tx) {
    return (
      <GlassPanel position="right" width="520px" title="TRANSACTION" onClose={handleClose}>
        <div className={styles.panelBody}>
          <span style={{ color: 'var(--rd-danger)' }}>
            {error ?? 'Transaction not found'}
          </span>
        </div>
      </GlassPanel>
    );
  }

  return (
    <GlassPanel
      position="right"
      width="520px"
      title="TRANSACTION"
      onClose={handleClose}
      active
      resizable
      ariaLabel={`Transaction ${tx.hash} detail`}
      testId="tx-detail"
    >
      <div className={styles.panelBody}>
        {/* ── Header: hash + status ── */}
        <div
          style={{
            display: 'flex',
            alignItems: 'center',
            justifyContent: 'space-between',
            marginBottom: 'var(--rd-space-md)',
          }}
        >
          <code
            style={{
              fontFamily: 'var(--rd-font-mono)',
              fontSize: 'var(--rd-text-sm)',
              color: 'var(--rd-text)',
            }}
            aria-label={`Transaction hash: ${tx.hash}`}
          >
            {truncateHash(tx.hash, 8)}
          </code>
          {receipt && <StatusBadge status={receipt.status} />}
        </div>

        {/* ── Flow diagram: From -> To ── */}
        <FlowDiagram
          from={tx.from}
          to={tx.to}
          value={tx.value}
          onAddressClick={goToAddress}
        />

        {/* ── Data grid ── */}
        <div className={styles.statsGrid} style={{ marginTop: 'var(--rd-space-md)' }}>
          <div className={styles.statCell}>
            <div className={styles.statLabel}>VALUE</div>
            <div className={styles.statValue}>{formatEth(tx.value)} ETH</div>
          </div>
          <div className={styles.statCell}>
            <div className={styles.statLabel}>GAS PRICE</div>
            <div className={styles.statValue}>{formatGas(tx.gasPrice)} gwei</div>
          </div>
          <div className={styles.statCell}>
            <div className={styles.statLabel}>GAS LIMIT</div>
            <div className={styles.statValue}>{formatGas(tx.gas)}</div>
          </div>
          <div className={styles.statCell}>
            <div className={styles.statLabel}>GAS USED</div>
            <div className={styles.statValue}>
              {receipt ? formatGas(receipt.gasUsed) : '--'}
            </div>
          </div>
          <div className={styles.statCell}>
            <div className={styles.statLabel}>NONCE</div>
            <div className={styles.statValue}>{tx.nonce}</div>
          </div>
          <div className={styles.statCell}>
            <div className={styles.statLabel}>TYPE</div>
            <div className={styles.statValue}>
              0x{tx.type.toString(16)}{' '}
              <span style={{ color: 'var(--rd-text-dim)' }}>
                ({tx.type === 2 ? '1559' : tx.type === 1 ? '2930' : 'legacy'})
              </span>
            </div>
          </div>
        </div>

        {/* ── Gas usage bar ── */}
        {receipt && (
          <div style={{ margin: 'var(--rd-space-md) 0' }}>
            <div className={styles.statLabel}>GAS USAGE</div>
            <div className={styles.gasBar} style={{ marginTop: 'var(--rd-space-xs)' }}>
              <div
                className={styles.gasBarFill}
                style={{
                  width: `${Number((receipt.gasUsed * 100n) / tx.gas)}%`,
                }}
              />
            </div>
          </div>
        )}

        {/* ── Input data ── */}
        {tx.input && tx.input !== '0x' && (
          <div style={{ marginTop: 'var(--rd-space-md)' }}>
            <div className={styles.statLabel}>INPUT DATA</div>
            <div
              style={{
                fontFamily: 'var(--rd-font-mono)',
                fontSize: 'var(--rd-text-xs)',
                color: 'var(--rd-text-dim)',
                background: 'var(--rd-void-light)',
                padding: 'var(--rd-space-sm)',
                marginTop: 'var(--rd-space-xs)',
                maxHeight: 120,
                overflow: 'auto',
                wordBreak: 'break-all',
              }}
            >
              {tx.input}
            </div>
          </div>
        )}

        {/* ── Block link ── */}
        <div
          style={{
            marginTop: 'var(--rd-space-md)',
            paddingTop: 'var(--rd-space-md)',
            borderTop: '1px solid var(--rd-border)',
          }}
        >
          <div className={styles.statLabel}>BLOCK</div>
          <button
            onClick={() => goToBlock(tx.blockNumber)}
            style={{
              background: 'none',
              border: 'none',
              color: 'var(--rd-rose)',
              cursor: 'pointer',
              fontFamily: 'var(--rd-font-mono)',
              fontSize: 'var(--rd-text-base)',
              padding: 0,
              textDecoration: 'none',
            }}
          >
            {formatNumber(tx.blockNumber)}
          </button>
        </div>

        {/* ── Receipt logs ── */}
        {receipt && receipt.logs.length > 0 && (
          <div style={{ marginTop: 'var(--rd-space-lg)' }}>
            <div
              className={styles.label}
              style={{
                marginBottom: 'var(--rd-space-sm)',
                letterSpacing: 'var(--rd-tracking-section)',
              }}
            >
              &mdash;&mdash; LOGS ({receipt.logs.length})
            </div>
            <div className={styles.scrollList}>
              {receipt.logs.map((log, i) => (
                <LogEntry key={`${log.transactionHash}-${log.logIndex}`} log={log} index={i} />
              ))}
            </div>
          </div>
        )}
      </div>
    </GlassPanel>
  );
}
```

### 9.3.2 Verification

- [ ] Header shows tx hash (truncated + copy button) and status badge (success/reverted)
- [ ] Flow diagram displays from -> to with address labels and value
- [ ] Clicking from/to addresses navigates to AddressDetail
- [ ] Stats show value, gas used/limit, gas price, nonce, tx type
- [ ] Input data displays in hex view with scrollable overflow
- [ ] Receipt logs are listed with topics and data
- [ ] Block number is clickable, navigates to BlockDetail
- [ ] Data fetched from `eth_getTransactionByHash` + `eth_getTransactionReceipt`

---

## 9.4 Address Detail Panel

Shown when an address is selected. Displays balance, type (EOA/contract), and
recent transactions from cached blocks.

### 9.4.1 `apps/explorer/src/panels/AddressDetail.tsx`

```tsx
import { useEffect, useState, useCallback } from 'react';
import { useNavigate, useParams } from 'react-router';
import { GlassPanel } from './GlassPanel';
import { useChainStore } from '@/data/store';
import { httpClient } from '@/data/rpc';
import {
  truncateHash,
  formatEth,
  formatGas,
  formatNumber,
} from '@/utils/format';
import type { ChainTransaction } from '@/data/types';
import styles from '@/design/glass.module.css';

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

interface AddressInfo {
  address: `0x${string}`;
  balance: bigint;
  isContract: boolean;
  codeSize: number;
  recentTxs: ChainTransaction[];
  loading: boolean;
  error: string | null;
}

// ---------------------------------------------------------------------------
// Data fetching
// ---------------------------------------------------------------------------

function useAddressData(address: string | undefined): AddressInfo {
  const blocks = useChainStore((s) => s.blocks);
  const [info, setInfo] = useState<AddressInfo>({
    address: (address ?? '0x') as `0x${string}`,
    balance: 0n,
    isContract: false,
    codeSize: 0,
    recentTxs: [],
    loading: true,
    error: null,
  });

  useEffect(() => {
    if (!address) return;
    let cancelled = false;

    async function fetch() {
      setInfo((prev) => ({ ...prev, loading: true, error: null }));

      try {
        // Fetch balance and code in parallel
        const [balanceHex, codeHex] = await Promise.all([
          httpClient.request({
            method: 'eth_getBalance',
            params: [address as `0x${string}`, 'latest'],
          }) as Promise<`0x${string}`>,
          httpClient.request({
            method: 'eth_getCode',
            params: [address as `0x${string}`, 'latest'],
          }) as Promise<`0x${string}`>,
        ]);

        // Scan cached blocks for recent transactions involving this address
        const recentTxs: ChainTransaction[] = [];
        const lowerAddr = address.toLowerCase();

        for (const block of blocks.values()) {
          if (!block.transactions) continue;
          for (const tx of block.transactions) {
            if (
              tx.from.toLowerCase() === lowerAddr ||
              (tx.to && tx.to.toLowerCase() === lowerAddr)
            ) {
              recentTxs.push(tx);
            }
          }
          // Limit to last 10
          if (recentTxs.length >= 10) break;
        }

        // Sort by block number descending
        recentTxs.sort((a, b) => {
          if (b.blockNumber > a.blockNumber) return 1;
          if (b.blockNumber < a.blockNumber) return -1;
          return b.transactionIndex - a.transactionIndex;
        });

        const balance = BigInt(balanceHex);
        const isContract = codeHex !== '0x';
        const codeSize = isContract ? (codeHex.length - 2) / 2 : 0; // hex chars / 2 = bytes

        if (!cancelled) {
          setInfo({
            address: address as `0x${string}`,
            balance,
            isContract,
            codeSize,
            recentTxs: recentTxs.slice(0, 10),
            loading: false,
            error: null,
          });
        }
      } catch (err) {
        if (!cancelled) {
          setInfo((prev) => ({
            ...prev,
            loading: false,
            error: err instanceof Error ? err.message : 'Failed to fetch address',
          }));
        }
      }
    }

    fetch();
    return () => {
      cancelled = true;
    };
  }, [address, blocks]);

  return info;
}

// ---------------------------------------------------------------------------
// Sub-components
// ---------------------------------------------------------------------------

function TypeBadge({ isContract }: { isContract: boolean }) {
  const cls = isContract ? styles.badgeContract : styles.badgeEoa;
  return (
    <span className={`${styles.badge} ${cls}`}>
      {isContract ? 'CONTRACT' : 'EOA'}
    </span>
  );
}

function TxRow({
  tx,
  highlightAddr,
  onTxClick,
}: {
  tx: ChainTransaction;
  highlightAddr: string;
  onTxClick: (hash: `0x${string}`) => void;
}) {
  const isSender = tx.from.toLowerCase() === highlightAddr.toLowerCase();

  return (
    <div
      className={styles.listRow}
      onClick={() => onTxClick(tx.hash)}
      role="button"
      tabIndex={0}
      onKeyDown={(e) => {
        if (e.key === 'Enter') onTxClick(tx.hash);
      }}
    >
      <code
        style={{
          fontFamily: 'var(--rd-font-mono)',
          fontSize: 'var(--rd-text-xs)',
          color: 'var(--rd-text-dim)',
          flex: '0 0 80px',
        }}
      >
        {truncateHash(tx.hash, 3)}
      </code>
      <span
        style={{
          fontFamily: 'var(--rd-font-mono)',
          fontSize: 'var(--rd-text-xs)',
          color: isSender ? 'var(--rd-danger)' : 'var(--rd-success)',
          flex: '0 0 30px',
        }}
      >
        {isSender ? 'OUT' : 'IN'}
      </span>
      <span
        style={{
          fontFamily: 'var(--rd-font-mono)',
          fontSize: 'var(--rd-text-xs)',
          color: 'var(--rd-text-dim)',
          flex: 1,
        }}
      >
        {isSender
          ? tx.to
            ? truncateHash(tx.to, 3)
            : 'CREATE'
          : truncateHash(tx.from, 3)}
      </span>
      <span
        style={{
          fontFamily: 'var(--rd-font-mono)',
          fontSize: 'var(--rd-text-sm)',
          color: 'var(--rd-bone)',
          flex: '0 0 90px',
          textAlign: 'right',
        }}
      >
        {formatEth(tx.value)} ETH
      </span>
      <span
        style={{
          fontFamily: 'var(--rd-font-mono)',
          fontSize: 'var(--rd-text-xs)',
          color: 'var(--rd-text-ghost)',
          flex: '0 0 60px',
          textAlign: 'right',
        }}
      >
        {formatGas(tx.gas)}
      </span>
    </div>
  );
}

// ---------------------------------------------------------------------------
// Main Component
// ---------------------------------------------------------------------------

export function AddressDetail() {
  const { address } = useParams<{ address: string }>();
  const navigate = useNavigate();
  const info = useAddressData(address);

  const handleClose = useCallback(() => navigate(-1), [navigate]);

  const goToTx = useCallback(
    (hash: `0x${string}`) => navigate(`/tx/${hash}`),
    [navigate],
  );

  // ── Loading ──
  if (info.loading) {
    return (
      <GlassPanel position="right" width="480px" title="ADDRESS" onClose={handleClose}>
        <div className={styles.panelBody}>
          <span style={{ color: 'var(--rd-text-dim)' }}>Loading...</span>
        </div>
      </GlassPanel>
    );
  }

  // ── Error ──
  if (info.error) {
    return (
      <GlassPanel position="right" width="480px" title="ADDRESS" onClose={handleClose}>
        <div className={styles.panelBody}>
          <span style={{ color: 'var(--rd-danger)' }}>{info.error}</span>
        </div>
      </GlassPanel>
    );
  }

  return (
    <GlassPanel
      position="right"
      width="480px"
      title="ADDRESS"
      onClose={handleClose}
      active
      resizable
      ariaLabel={`Address ${info.address} detail`}
      testId="address-detail"
    >
      <div className={styles.panelBody}>
        {/* ── Header: address + type badge ── */}
        <div
          style={{
            display: 'flex',
            alignItems: 'center',
            justifyContent: 'space-between',
            marginBottom: 'var(--rd-space-md)',
          }}
        >
          <code
            style={{
              fontFamily: 'var(--rd-font-mono)',
              fontSize: 'var(--rd-text-sm)',
              color: 'var(--rd-text)',
              wordBreak: 'break-all',
            }}
            aria-label={`Address: ${info.address}`}
          >
            {info.address}
          </code>
          <div style={{ marginLeft: 'var(--rd-space-sm)', flexShrink: 0 }}>
            <TypeBadge isContract={info.isContract} />
          </div>
        </div>

        {/* ── Balance (hero number) ── */}
        <div style={{ marginBottom: 'var(--rd-space-lg)' }}>
          <div className={styles.statLabel}>BALANCE</div>
          <div
            style={{
              fontFamily: 'var(--rd-font-display)',
              fontStyle: 'italic',
              fontSize: 'var(--rd-text-hero)',
              color: 'var(--rd-bone-bright)',
              lineHeight: 1.1,
            }}
          >
            {formatEth(info.balance)}{' '}
            <span
              style={{
                fontSize: 'var(--rd-text-lg)',
                color: 'var(--rd-text-dim)',
              }}
            >
              ETH
            </span>
          </div>
        </div>

        {/* ── Contract info (if contract) ── */}
        {info.isContract && (
          <div className={styles.statsGrid} style={{ marginBottom: 'var(--rd-space-md)' }}>
            <div className={styles.statCell}>
              <div className={styles.statLabel}>CODE SIZE</div>
              <div className={styles.statValue}>
                {formatNumber(BigInt(info.codeSize))}{' '}
                <span style={{ color: 'var(--rd-text-dim)' }}>bytes</span>
              </div>
            </div>
            <div className={styles.statCell}>
              <div className={styles.statLabel}>TYPE</div>
              <div className={styles.statValue} style={{ color: 'var(--rd-rose)' }}>
                Contract
              </div>
            </div>
          </div>
        )}

        {/* ── Recent transactions ── */}
        <div style={{ marginTop: 'var(--rd-space-md)' }}>
          <div
            className={styles.label}
            style={{
              marginBottom: 'var(--rd-space-sm)',
              letterSpacing: 'var(--rd-tracking-section)',
            }}
          >
            &mdash;&mdash; RECENT TRANSACTIONS
          </div>

          {info.recentTxs.length === 0 ? (
            <div
              style={{
                fontFamily: 'var(--rd-font-mono)',
                fontSize: 'var(--rd-text-sm)',
                color: 'var(--rd-text-ghost)',
                padding: 'var(--rd-space-md)',
                textAlign: 'center',
              }}
            >
              No transactions found in cached blocks
            </div>
          ) : (
            <div className={styles.scrollList}>
              {/* Table header */}
              <div
                style={{
                  display: 'flex',
                  padding: 'var(--rd-space-xs) var(--rd-space-md)',
                  borderBottom: '1px solid var(--rd-border-strong)',
                }}
              >
                <span className={styles.label} style={{ flex: '0 0 80px' }}>HASH</span>
                <span className={styles.label} style={{ flex: '0 0 30px' }}>DIR</span>
                <span className={styles.label} style={{ flex: 1 }}>TO/FROM</span>
                <span className={styles.label} style={{ flex: '0 0 90px', textAlign: 'right' }}>VALUE</span>
                <span className={styles.label} style={{ flex: '0 0 60px', textAlign: 'right' }}>GAS</span>
              </div>
              {info.recentTxs.map((tx) => (
                <TxRow
                  key={tx.hash}
                  tx={tx}
                  highlightAddr={info.address}
                  onTxClick={goToTx}
                />
              ))}
            </div>
          )}
        </div>
      </div>
    </GlassPanel>
  );
}
```

### 9.4.2 Verification

- [ ] Header shows full address (copyable) with type badge (EOA/Contract)
- [ ] Balance shown in Fraunces italic hero style
- [ ] Type determined by `eth_getCode` result (non-`0x` = contract)
- [ ] Contract info shows code size if contract
- [ ] Recent transactions list: last 10 from cached blocks involving this address
- [ ] Each transaction row shows direction (IN/OUT), hash, counterparty, value, gas
- [ ] Clicking a transaction navigates to TxDetail
- [ ] Data from `eth_getBalance`, `eth_getCode`, cached block scan

---

## 9.5 Search Overlay

Full-width search bar triggered by Cmd+K / Ctrl+K or clicking a search icon.
Local-first with RPC fallback. Keyboard navigable.

### 9.5.1 `apps/explorer/src/panels/SearchOverlay.tsx`

```tsx
import {
  useState,
  useCallback,
  useEffect,
  useRef,
  useMemo,
} from 'react';
import { useNavigate } from 'react-router';
import { motion, AnimatePresence } from 'framer-motion';
import { useChainStore, useUIStore } from '@/data/store';
import { httpClient } from '@/data/rpc';
import { truncateHash, formatEth, formatNumber } from '@/utils/format';
import styles from '@/design/glass.module.css';

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

interface SearchResult {
  type: 'block' | 'transaction' | 'address';
  label: string;
  subtitle: string;
  source: 'cache' | 'rpc';
  navigateTo: string;
}

// Key for localStorage recent searches
const RECENT_SEARCHES_KEY = 'kora-explorer-recent-searches';

// ---------------------------------------------------------------------------
// Query parsing
// ---------------------------------------------------------------------------

type QueryKind = 'block_number' | 'tx_hash' | 'address' | 'invalid';

function classifyQuery(query: string): QueryKind {
  const trimmed = query.trim();
  if (!trimmed) return 'invalid';

  // Pure decimal number -> block number
  if (/^\d+$/.test(trimmed)) return 'block_number';

  // 0x + 64 hex chars -> tx hash
  if (/^0x[0-9a-fA-F]{64}$/.test(trimmed)) return 'tx_hash';

  // 0x + 40 hex chars -> address
  if (/^0x[0-9a-fA-F]{40}$/.test(trimmed)) return 'address';

  // Partial hex -> could be either, but not valid enough to search
  if (/^0x[0-9a-fA-F]+$/.test(trimmed)) {
    const hexLen = trimmed.length - 2;
    if (hexLen < 40) return 'invalid';
    if (hexLen === 40) return 'address';
    if (hexLen === 64) return 'tx_hash';
    return 'invalid';
  }

  return 'invalid';
}

// ---------------------------------------------------------------------------
// Recent searches (localStorage)
// ---------------------------------------------------------------------------

function getRecentSearches(): string[] {
  try {
    const raw = localStorage.getItem(RECENT_SEARCHES_KEY);
    if (!raw) return [];
    return JSON.parse(raw) as string[];
  } catch {
    return [];
  }
}

function addRecentSearch(query: string) {
  const recent = getRecentSearches().filter((q) => q !== query);
  recent.unshift(query);
  const trimmed = recent.slice(0, 8); // keep last 8
  localStorage.setItem(RECENT_SEARCHES_KEY, JSON.stringify(trimmed));
}

// ---------------------------------------------------------------------------
// Search hook
// ---------------------------------------------------------------------------

function useSearch() {
  const blocks = useChainStore((s) => s.blocks);
  const [results, setResults] = useState<SearchResult[]>([]);
  const [searching, setSearching] = useState(false);
  const [hint, setHint] = useState<string | null>(null);
  const debounceRef = useRef<ReturnType<typeof setTimeout>>();

  const search = useCallback(
    (query: string) => {
      // Clear pending debounce
      clearTimeout(debounceRef.current);

      const trimmed = query.trim();
      if (!trimmed) {
        setResults([]);
        setHint(null);
        setSearching(false);
        return;
      }

      const kind = classifyQuery(trimmed);

      if (kind === 'invalid') {
        setResults([]);
        setHint('Enter a block number, transaction hash (0x + 64 hex), or address (0x + 40 hex)');
        setSearching(false);
        return;
      }

      setHint(null);

      // ── Instant cache search ──
      if (kind === 'block_number') {
        const num = BigInt(trimmed);
        const cached = blocks.get(num);
        if (cached) {
          setResults([
            {
              type: 'block',
              label: `Block ${formatNumber(cached.number)}`,
              subtitle: `${cached.transactions?.length ?? 0} txns`,
              source: 'cache',
              navigateTo: `/block/${cached.number}`,
            },
          ]);
          setSearching(false);
          return;
        }
      }

      if (kind === 'address') {
        // Address: navigate directly, no search needed
        setResults([
          {
            type: 'address',
            label: truncateHash(trimmed, 6),
            subtitle: 'View address',
            source: 'cache',
            navigateTo: `/address/${trimmed}`,
          },
        ]);
        setSearching(false);
        return;
      }

      // ── RPC fallback (debounced 150ms) ──
      setSearching(true);

      debounceRef.current = setTimeout(async () => {
        try {
          if (kind === 'block_number') {
            const hex = `0x${BigInt(trimmed).toString(16)}`;
            const block = await httpClient.request({
              method: 'eth_getBlockByNumber',
              params: [hex as `0x${string}`, false],
            });

            if (block) {
              setResults([
                {
                  type: 'block',
                  label: `Block ${formatNumber(BigInt(trimmed))}`,
                  subtitle: 'Fetched from RPC',
                  source: 'rpc',
                  navigateTo: `/block/${trimmed}`,
                },
              ]);
            } else {
              setResults([]);
              setHint('Block not found');
            }
          } else if (kind === 'tx_hash') {
            const tx = (await httpClient.request({
              method: 'eth_getTransactionByHash',
              params: [trimmed as `0x${string}`],
            })) as { hash: `0x${string}`; value: `0x${string}` } | null;

            if (tx) {
              setResults([
                {
                  type: 'transaction',
                  label: truncateHash(tx.hash, 6),
                  subtitle: `${formatEth(BigInt(tx.value))} ETH`,
                  source: 'rpc',
                  navigateTo: `/tx/${tx.hash}`,
                },
              ]);
            } else {
              setResults([]);
              setHint('Transaction not found');
            }
          }
        } catch {
          setHint('RPC error -- check connection');
          setResults([]);
        } finally {
          setSearching(false);
        }
      }, 150);
    },
    [blocks],
  );

  return { results, searching, hint, search };
}

// ---------------------------------------------------------------------------
// Result type icon
// ---------------------------------------------------------------------------

function ResultIcon({ type }: { type: SearchResult['type'] }) {
  const color =
    type === 'block'
      ? 'var(--rd-rose)'
      : type === 'transaction'
        ? 'var(--rd-bone-bright)'
        : 'var(--rd-dream)';

  return (
    <span
      style={{
        width: 8,
        height: 8,
        borderRadius: type === 'address' ? '50%' : 0,
        background: color,
        display: 'inline-block',
        flexShrink: 0,
      }}
    />
  );
}

// ---------------------------------------------------------------------------
// Main Component
// ---------------------------------------------------------------------------

export function SearchOverlay() {
  const searchOpen = useUIStore((s) => s.searchOpen);
  const setSearchOpen = useUIStore((s) => s.setSearchOpen);
  const navigate = useNavigate();

  const [query, setQuery] = useState('');
  const [selectedIndex, setSelectedIndex] = useState(0);
  const inputRef = useRef<HTMLInputElement>(null);
  const { results, searching, hint, search } = useSearch();

  // ── Keyboard shortcut: Cmd+K / Ctrl+K to open, / to open ──
  useEffect(() => {
    const handler = (e: KeyboardEvent) => {
      const isCmdK = (e.metaKey || e.ctrlKey) && e.key === 'k';
      const isSlash = e.key === '/' && !e.ctrlKey && !e.metaKey;

      // Do not capture / if user is focused on an input
      if (isSlash && (e.target instanceof HTMLInputElement || e.target instanceof HTMLTextAreaElement)) {
        return;
      }

      if (isCmdK || isSlash) {
        e.preventDefault();
        setSearchOpen(true);
      }
    };
    window.addEventListener('keydown', handler);
    return () => window.removeEventListener('keydown', handler);
  }, [setSearchOpen]);

  // ── Auto-focus on open ──
  useEffect(() => {
    if (searchOpen) {
      // Small delay to let animation start
      requestAnimationFrame(() => inputRef.current?.focus());
    } else {
      setQuery('');
      setSelectedIndex(0);
    }
  }, [searchOpen]);

  // ── Run search on query change ──
  useEffect(() => {
    search(query);
    setSelectedIndex(0);
  }, [query, search]);

  // ── Close handler ──
  const close = useCallback(() => {
    setSearchOpen(false);
    setQuery('');
  }, [setSearchOpen]);

  // ── Select result ──
  const selectResult = useCallback(
    (result: SearchResult) => {
      addRecentSearch(query);
      close();
      navigate(result.navigateTo);
    },
    [query, close, navigate],
  );

  // ── Keyboard navigation within overlay ──
  const handleKeyDown = useCallback(
    (e: React.KeyboardEvent) => {
      if (e.key === 'Escape') {
        e.preventDefault();
        close();
      } else if (e.key === 'ArrowDown') {
        e.preventDefault();
        setSelectedIndex((i) => Math.min(i + 1, results.length - 1));
      } else if (e.key === 'ArrowUp') {
        e.preventDefault();
        setSelectedIndex((i) => Math.max(i - 1, 0));
      } else if (e.key === 'Enter' && results[selectedIndex]) {
        e.preventDefault();
        selectResult(results[selectedIndex]);
      }
    },
    [results, selectedIndex, close, selectResult],
  );

  // ── Recent searches for empty state ──
  const recentSearches = useMemo(() => getRecentSearches(), [searchOpen]);

  if (!searchOpen) return null;

  return (
    <AnimatePresence>
      <motion.div
        key="search-overlay"
        initial={{ opacity: 0 }}
        animate={{ opacity: 1 }}
        exit={{ opacity: 0 }}
        transition={{ duration: 0.15 }}
        style={{
          position: 'fixed',
          inset: 0,
          zIndex: 'var(--rd-z-search)' as unknown as number,
          display: 'flex',
          flexDirection: 'column',
          alignItems: 'center',
          paddingTop: 120,
        }}
        onClick={(e) => {
          // Close on backdrop click
          if (e.target === e.currentTarget) close();
        }}
      >
        {/* Backdrop dim */}
        <div
          style={{
            position: 'absolute',
            inset: 0,
            background: 'rgba(6, 6, 8, 0.6)',
          }}
        />

        {/* Search container */}
        <motion.div
          initial={{ y: -12, opacity: 0 }}
          animate={{ y: 0, opacity: 1 }}
          exit={{ y: -12, opacity: 0 }}
          transition={{ duration: 0.2, ease: [0.16, 1, 0.3, 1] }}
          className={styles.panel}
          style={{
            position: 'relative',
            width: '100%',
            maxWidth: 640,
            margin: '0 var(--rd-space-lg)',
            overflow: 'hidden',
          }}
          role="combobox"
          aria-expanded={results.length > 0}
          aria-haspopup="listbox"
        >
          {/* ── Input ── */}
          <div
            style={{
              display: 'flex',
              alignItems: 'center',
              padding: 'var(--rd-space-md)',
              gap: 'var(--rd-space-sm)',
              borderBottom: '1px solid var(--rd-border)',
            }}
          >
            {/* Search icon */}
            <span
              style={{
                color: 'var(--rd-text-dim)',
                fontSize: 'var(--rd-text-lg)',
                flexShrink: 0,
              }}
            >
              &#9906;
            </span>

            <input
              ref={inputRef}
              type="text"
              value={query}
              onChange={(e) => setQuery(e.target.value)}
              onKeyDown={handleKeyDown}
              placeholder="Search blocks, transactions, addresses..."
              aria-label="Search blocks, transactions, addresses"
              style={{
                flex: 1,
                background: 'none',
                border: 'none',
                outline: 'none',
                fontFamily: 'var(--rd-font-mono)',
                fontSize: 'var(--rd-text-base)',
                color: 'var(--rd-text)',
                letterSpacing: 'var(--rd-tracking-normal)',
              }}
            />

            {/* Loading indicator */}
            {searching && (
              <span
                style={{
                  color: 'var(--rd-text-dim)',
                  fontSize: 'var(--rd-text-xs)',
                  fontFamily: 'var(--rd-font-mono)',
                }}
              >
                ...
              </span>
            )}

            {/* Close shortcut hint */}
            <span
              style={{
                fontFamily: 'var(--rd-font-mono)',
                fontSize: 'var(--rd-text-xs)',
                color: 'var(--rd-text-ghost)',
                border: '1px solid var(--rd-border)',
                padding: '1px 4px',
                flexShrink: 0,
              }}
            >
              ESC
            </span>
          </div>

          {/* ── Results list ── */}
          {results.length > 0 && (
            <div role="listbox" aria-label="Search results">
              {results.map((result, i) => (
                <div
                  key={`${result.type}-${result.navigateTo}`}
                  role="option"
                  aria-selected={i === selectedIndex}
                  className={styles.listRow}
                  style={{
                    background:
                      i === selectedIndex ? 'var(--rd-glass-bg-hover)' : 'transparent',
                  }}
                  onClick={() => selectResult(result)}
                  onMouseEnter={() => setSelectedIndex(i)}
                >
                  <ResultIcon type={result.type} />
                  <span className={styles.label} style={{ flex: '0 0 auto' }}>
                    {result.type.toUpperCase()}
                  </span>
                  <code
                    style={{
                      fontFamily: 'var(--rd-font-mono)',
                      fontSize: 'var(--rd-text-sm)',
                      color: 'var(--rd-text)',
                      flex: 1,
                    }}
                  >
                    {result.label}
                  </code>
                  <span
                    style={{
                      fontFamily: 'var(--rd-font-mono)',
                      fontSize: 'var(--rd-text-xs)',
                      color: 'var(--rd-text-dim)',
                    }}
                  >
                    {result.subtitle}
                  </span>
                </div>
              ))}
            </div>
          )}

          {/* ── Hint (invalid query / not found) ── */}
          {hint && (
            <div
              style={{
                padding: 'var(--rd-space-md)',
                fontFamily: 'var(--rd-font-mono)',
                fontSize: 'var(--rd-text-sm)',
                color: 'var(--rd-text-dim)',
                textAlign: 'center',
              }}
            >
              {hint}
            </div>
          )}

          {/* ── Recent searches (when input is empty) ── */}
          {!query && recentSearches.length > 0 && (
            <div style={{ padding: 'var(--rd-space-sm) 0' }}>
              <div
                className={styles.label}
                style={{ padding: 'var(--rd-space-xs) var(--rd-space-md)' }}
              >
                RECENT
              </div>
              {recentSearches.map((recent) => (
                <div
                  key={recent}
                  className={styles.listRow}
                  onClick={() => setQuery(recent)}
                  role="button"
                  tabIndex={0}
                  onKeyDown={(e) => {
                    if (e.key === 'Enter') setQuery(recent);
                  }}
                >
                  <code
                    style={{
                      fontFamily: 'var(--rd-font-mono)',
                      fontSize: 'var(--rd-text-sm)',
                      color: 'var(--rd-text-dim)',
                    }}
                  >
                    {recent}
                  </code>
                </div>
              ))}
            </div>
          )}
        </motion.div>
      </motion.div>
    </AnimatePresence>
  );
}
```

### 9.5.2 UI Store additions

The `SearchOverlay` needs `searchOpen` and `setSearchOpen` on the UI store.
These were defined in doc 04 (`useUIStore`). For reference:

```tsx
// In apps/explorer/src/data/store.ts — UIStore additions (if not already present)
searchOpen: false,
setSearchOpen: (open: boolean) => set({ searchOpen: open }),
```

### 9.5.3 Verification

- [ ] Cmd+K / Ctrl+K opens the overlay, `/` also opens it (unless focused in input)
- [ ] Input field has monospace font, placeholder text visible
- [ ] Parsing: number -> block number, 0x+64 hex -> tx hash, 0x+40 hex -> address
- [ ] Cache matches appear instantly (no debounce)
- [ ] RPC fallback fires after 150ms debounce
- [ ] "invalid query" hint shown for unrecognized input
- [ ] ArrowUp/ArrowDown select results, Enter confirms selection
- [ ] Escape closes overlay
- [ ] Clicking backdrop closes overlay
- [ ] Recent searches (last 8) stored in localStorage, shown when input is empty
- [ ] Selected result has rose-tinted hover background
- [ ] Results have `role="listbox"` with `aria-activedescendant`

---

## 9.6 Status Bar

Fixed bottom bar. Always visible. Shows connection status, current block number,
and optional dev-mode FPS counter.

### 9.6.1 `apps/explorer/src/panels/StatusBar.tsx`

```tsx
import { useChainStore, useUIStore } from '@/data/store';
import { formatNumber } from '@/utils/format';
import { useFrameMonitor } from '@/hooks/useFrameMonitor';
import styles from '@/design/glass.module.css';

// ---------------------------------------------------------------------------
// Connection LED
// ---------------------------------------------------------------------------

function ConnectionLed() {
  const status = useChainStore((s) => s.status);

  const ledColor =
    status === 'connected'
      ? 'var(--rd-led-connected)'
      : status === 'reconnecting'
        ? 'var(--rd-led-warning)'
        : 'var(--rd-led-error)';

  const label =
    status === 'connected'
      ? 'Connected'
      : status === 'reconnecting'
        ? 'Reconnecting...'
        : 'Disconnected';

  return (
    <div
      style={{
        display: 'flex',
        alignItems: 'center',
        gap: 'var(--rd-space-xs)',
      }}
      aria-live="polite"
    >
      <span
        className={styles.led}
        style={{
          background: ledColor,
          boxShadow: `0 0 6px ${ledColor}`,
          animation: status === 'reconnecting' ? 'pulse 1.2s ease-in-out infinite' : undefined,
        }}
      />
      <span className={styles.label}>{label}</span>
    </div>
  );
}

// ---------------------------------------------------------------------------
// Block number + block time
// ---------------------------------------------------------------------------

function BlockCounter() {
  const latestBlockNumber = useChainStore((s) => s.latestBlockNumber);
  const latestBlock = useChainStore((s) => s.latestBlock);

  // Estimate block time from last two blocks
  let blockTimeLabel = '';
  if (latestBlock && latestBlock._arrivalTime) {
    const elapsed = Date.now() - latestBlock._arrivalTime;
    if (elapsed < 60_000) {
      blockTimeLabel = `${Math.round(elapsed / 1000)}s ago`;
    }
  }

  return (
    <div
      style={{
        display: 'flex',
        alignItems: 'center',
        gap: 'var(--rd-space-sm)',
      }}
    >
      <span
        style={{
          fontFamily: 'var(--rd-font-mono)',
          fontSize: 'var(--rd-text-sm)',
          color: 'var(--rd-rose)',
          letterSpacing: 'var(--rd-tracking-normal)',
        }}
      >
        #{formatNumber(latestBlockNumber)}
      </span>
      {blockTimeLabel && (
        <span
          style={{
            fontFamily: 'var(--rd-font-mono)',
            fontSize: 'var(--rd-text-xs)',
            color: 'var(--rd-text-dim)',
          }}
        >
          {blockTimeLabel}
        </span>
      )}
    </div>
  );
}

// ---------------------------------------------------------------------------
// FPS counter (dev mode only)
// ---------------------------------------------------------------------------

function FpsCounter() {
  const fps = useFrameMonitor();

  // Only show in dev mode
  if (import.meta.env.PROD) return null;

  const fpsColor =
    fps >= 55
      ? 'var(--rd-success)'
      : fps >= 30
        ? 'var(--rd-warning)'
        : 'var(--rd-danger)';

  return (
    <span
      style={{
        fontFamily: 'var(--rd-font-mono)',
        fontSize: 'var(--rd-text-xs)',
        color: fpsColor,
        letterSpacing: 'var(--rd-tracking-normal)',
      }}
    >
      {fps} FPS
    </span>
  );
}

// ---------------------------------------------------------------------------
// Performance tier badge
// ---------------------------------------------------------------------------

function TierBadge() {
  // Tier is determined at startup and stored in UI store
  // For now, show a static badge. Real implementation reads from useUIStore.
  return (
    <span
      className={styles.label}
      style={{
        border: '1px solid var(--rd-border)',
        padding: '1px 4px',
        fontSize: 'var(--rd-text-xs)',
      }}
    >
      FULL
    </span>
  );
}

// ---------------------------------------------------------------------------
// Main Component
// ---------------------------------------------------------------------------

export function StatusBar() {
  const ambientMode = useUIStore((s) => s.ambientMode);

  return (
    <div
      style={{
        position: 'fixed',
        left: 0,
        right: 0,
        bottom: 0,
        height: 32,
        background: 'var(--rd-void-surface)',
        borderTop: '1px solid var(--rd-border)',
        display: 'flex',
        alignItems: 'center',
        justifyContent: 'space-between',
        padding: '0 var(--rd-space-md)',
        zIndex: 'var(--rd-z-panels)' as unknown as number,
        opacity: ambientMode ? 0.2 : 1,
        transition: 'opacity 2s ease-out',
        pointerEvents: 'auto',
      }}
      role="status"
      aria-label="Explorer status bar"
    >
      {/* Left: connection status */}
      <ConnectionLed />

      {/* Center: block number + block time */}
      <BlockCounter />

      {/* Right: FPS + tier */}
      <div
        style={{
          display: 'flex',
          alignItems: 'center',
          gap: 'var(--rd-space-sm)',
        }}
      >
        <FpsCounter />
        <TierBadge />
      </div>
    </div>
  );
}
```

### 9.6.2 `apps/explorer/src/hooks/useFrameMonitor.ts`

```tsx
import { useState, useEffect, useRef } from 'react';

/**
 * Tracks frame rate using requestAnimationFrame.
 * Returns the current FPS as a rounded integer.
 * Only updates the React state once per second to avoid thrashing.
 */
export function useFrameMonitor(): number {
  const [fps, setFps] = useState(60);
  const framesRef = useRef(0);
  const lastRef = useRef(performance.now());
  const rafRef = useRef<number>();

  useEffect(() => {
    // Only run in dev mode
    if (import.meta.env.PROD) return;

    const tick = (now: number) => {
      framesRef.current++;

      const elapsed = now - lastRef.current;
      if (elapsed >= 1000) {
        setFps(Math.round((framesRef.current * 1000) / elapsed));
        framesRef.current = 0;
        lastRef.current = now;
      }

      rafRef.current = requestAnimationFrame(tick);
    };

    rafRef.current = requestAnimationFrame(tick);
    return () => {
      if (rafRef.current) cancelAnimationFrame(rafRef.current);
    };
  }, []);

  return fps;
}
```

### 9.6.3 Verification

- [ ] StatusBar is 32px tall, fixed to bottom, always visible
- [ ] Left: green dot + "Connected" when connected, red + "Disconnected" when not
- [ ] Center: current block number updates live when new blocks arrive
- [ ] Block time estimate shows "Xs ago" since last block
- [ ] Right: FPS counter visible in dev mode only, hidden in production
- [ ] Performance tier badge shows current tier
- [ ] Uses `--rd-void-surface` background, `--rd-text-dim` text
- [ ] Connection LED has `aria-live="polite"` for screen readers
- [ ] Panel fades to 20% opacity in ambient mode

---

## 9.7 Scene Navigation Tabs

Tab bar for switching between the four scenes. Floating pills at top-left.

### 9.7.1 `apps/explorer/src/panels/SceneNav.tsx`

```tsx
import { useCallback, useEffect } from 'react';
import { useUIStore } from '@/data/store';
import styles from '@/design/glass.module.css';

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

type SceneId = 'terrain' | 'constellation' | 'waterfall' | 'consensus';

interface SceneTab {
  id: SceneId;
  label: string;
  key: string; // keyboard shortcut
}

const SCENE_TABS: SceneTab[] = [
  { id: 'terrain', label: 'TERRAIN', key: '1' },
  { id: 'constellation', label: 'CONSTELLATION', key: '2' },
  { id: 'waterfall', label: 'WATERFALL', key: '3' },
  { id: 'consensus', label: 'CONSENSUS', key: '4' },
];

// ---------------------------------------------------------------------------
// Component
// ---------------------------------------------------------------------------

export function SceneNav() {
  const activeScene = useUIStore((s) => s.activeScene);
  const setActiveScene = useUIStore((s) => s.setActiveScene);
  const ambientMode = useUIStore((s) => s.ambientMode);

  // ── Keyboard: 1-4 switch scenes ──
  useEffect(() => {
    const handler = (e: KeyboardEvent) => {
      // Don't capture if user is typing in an input
      if (e.target instanceof HTMLInputElement || e.target instanceof HTMLTextAreaElement) {
        return;
      }

      const tab = SCENE_TABS.find((t) => t.key === e.key);
      if (tab) {
        e.preventDefault();
        setActiveScene(tab.id);
      }
    };
    window.addEventListener('keydown', handler);
    return () => window.removeEventListener('keydown', handler);
  }, [setActiveScene]);

  const handleClick = useCallback(
    (id: SceneId) => {
      setActiveScene(id);
    },
    [setActiveScene],
  );

  return (
    <nav
      role="tablist"
      aria-label="Scene navigation"
      style={{
        position: 'fixed',
        top: 'var(--rd-space-lg)',
        left: 'var(--rd-space-lg)',
        display: 'flex',
        gap: 1,
        zIndex: 'var(--rd-z-panels)' as unknown as number,
        opacity: ambientMode ? 0.2 : 1,
        transition: 'opacity 2s ease-out',
        pointerEvents: 'auto',
      }}
    >
      {SCENE_TABS.map((tab) => {
        const isActive = activeScene === tab.id;

        return (
          <button
            key={tab.id}
            role="tab"
            aria-selected={isActive}
            onClick={() => handleClick(tab.id)}
            title={`${tab.label} (${tab.key})`}
            style={{
              background: isActive ? 'var(--rd-glass-bg)' : 'transparent',
              border: '1px solid var(--rd-border)',
              borderBottom: isActive
                ? '2px solid var(--rd-rose)'
                : '1px solid var(--rd-border)',
              color: isActive ? 'var(--rd-text)' : 'var(--rd-text-dim)',
              fontFamily: 'var(--rd-font-mono)',
              fontSize: 'var(--rd-text-xs)',
              textTransform: 'uppercase',
              letterSpacing: 'var(--rd-tracking-label)',
              padding: 'var(--rd-space-xs) var(--rd-space-md)',
              cursor: 'pointer',
              backdropFilter: isActive
                ? 'blur(var(--rd-glass-blur))'
                : 'none',
              WebkitBackdropFilter: isActive
                ? 'blur(var(--rd-glass-blur))'
                : 'none',
              transition: `
                color var(--rd-duration-fast) var(--rd-ease-out),
                background var(--rd-duration-fast) var(--rd-ease-out),
                border-color var(--rd-duration-fast) var(--rd-ease-out)
              `,
            }}
          >
            {tab.label}
            <span
              style={{
                marginLeft: 'var(--rd-space-xs)',
                color: 'var(--rd-text-ghost)',
                fontSize: 'var(--rd-text-xs)',
              }}
            >
              {tab.key}
            </span>
          </button>
        );
      })}
    </nav>
  );
}
```

### 9.7.2 UI Store additions

```tsx
// In apps/explorer/src/data/store.ts — UIStore (if not already present)
activeScene: 'terrain' as 'terrain' | 'constellation' | 'waterfall' | 'consensus',
setActiveScene: (scene) => set({ activeScene: scene }),
```

### 9.7.3 Verification

- [ ] 4 tabs: Terrain, Constellation, Waterfall, Consensus
- [ ] Active tab has `--rd-accent` (rose) underline and glass background
- [ ] Inactive tabs are transparent with dim text
- [ ] Keyboard: 1-4 number keys switch scenes (not captured when typing in input)
- [ ] Position: fixed top-left as floating pills
- [ ] Tabs use `role="tablist"` with `aria-selected` on active tab
- [ ] Tabs fade to 20% in ambient mode

---

## 9.8 Tooltip Component

Generic tooltip for hover info. Shows entity summary on mouse hover over scene elements.

### 9.8.1 `apps/explorer/src/panels/Tooltip.tsx`

```tsx
import { useState, useEffect, useRef, useCallback, type ReactNode } from 'react';
import { bus } from '@/data/bus';
import { truncateHash, formatEth, formatNumber } from '@/utils/format';
import styles from '@/design/glass.module.css';

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

interface TooltipData {
  type: 'block' | 'transaction' | 'address';
  x: number;
  y: number;
  content: {
    primary: string;     // e.g., block number or hash
    secondary?: string;  // e.g., tx count or balance
    tertiary?: string;   // e.g., timestamp or gas
  };
}

// Viewport edge clamping constants
const TOOLTIP_OFFSET_X = 12;
const TOOLTIP_OFFSET_Y = -8;
const TOOLTIP_MAX_WIDTH = 240;
const VIEWPORT_PADDING = 16;

// ---------------------------------------------------------------------------
// Event-driven tooltip state
// ---------------------------------------------------------------------------

/**
 * Scenes emit 'tooltip:show' and 'tooltip:hide' events on the bus.
 * This component listens imperatively (not through React re-renders on the scene side).
 *
 * Bus event additions for doc 04:
 *   'tooltip:show': TooltipData
 *   'tooltip:hide': void
 */

export function Tooltip() {
  const [data, setData] = useState<TooltipData | null>(null);
  const [visible, setVisible] = useState(false);
  const tooltipRef = useRef<HTMLDivElement>(null);
  const hideTimerRef = useRef<ReturnType<typeof setTimeout>>();

  // ── Listen to bus events ──
  useEffect(() => {
    const showHandler = (payload: TooltipData) => {
      clearTimeout(hideTimerRef.current);
      setData(payload);
      setVisible(true);
    };

    const hideHandler = () => {
      // Small delay to prevent flicker when moving between adjacent elements
      hideTimerRef.current = setTimeout(() => {
        setVisible(false);
      }, 80);
    };

    // Type assertion because tooltip events are additions to BusEvents
    (bus as any).on('tooltip:show', showHandler);
    (bus as any).on('tooltip:hide', hideHandler);

    return () => {
      (bus as any).off('tooltip:show', showHandler);
      (bus as any).off('tooltip:hide', hideHandler);
      clearTimeout(hideTimerRef.current);
    };
  }, []);

  // ── Position clamping ──
  const getClampedPosition = useCallback((): React.CSSProperties => {
    if (!data) return { display: 'none' };

    let x = data.x + TOOLTIP_OFFSET_X;
    let y = data.y + TOOLTIP_OFFSET_Y;

    // Clamp right edge
    if (x + TOOLTIP_MAX_WIDTH > window.innerWidth - VIEWPORT_PADDING) {
      x = data.x - TOOLTIP_MAX_WIDTH - TOOLTIP_OFFSET_X;
    }

    // Clamp left edge
    if (x < VIEWPORT_PADDING) {
      x = VIEWPORT_PADDING;
    }

    // Clamp bottom (approximate tooltip height as 60px)
    if (y + 60 > window.innerHeight - VIEWPORT_PADDING) {
      y = data.y - 60 - TOOLTIP_OFFSET_Y;
    }

    // Clamp top
    if (y < VIEWPORT_PADDING) {
      y = VIEWPORT_PADDING;
    }

    return {
      left: x,
      top: y,
    };
  }, [data]);

  if (!visible || !data) return null;

  // ── Color by type ──
  const accentColor =
    data.type === 'block'
      ? 'var(--rd-rose)'
      : data.type === 'transaction'
        ? 'var(--rd-bone-bright)'
        : 'var(--rd-dream)';

  return (
    <div
      ref={tooltipRef}
      className={styles.panel}
      style={{
        position: 'fixed',
        ...getClampedPosition(),
        maxWidth: TOOLTIP_MAX_WIDTH,
        padding: 'var(--rd-space-sm) var(--rd-space-md)',
        zIndex: 'var(--rd-z-overlay)' as unknown as number,
        pointerEvents: 'none',
        opacity: visible ? 1 : 0,
        transform: visible ? 'translateY(0)' : 'translateY(4px)',
        transition: `
          opacity var(--rd-duration-normal) var(--rd-ease-out),
          transform var(--rd-duration-normal) var(--rd-ease-out)
        `,
        borderLeft: `2px solid ${accentColor}`,
      }}
    >
      {/* Primary label */}
      <div
        style={{
          fontFamily: 'var(--rd-font-mono)',
          fontSize: 'var(--rd-text-xs)',
          color: accentColor,
          textTransform: 'uppercase',
          letterSpacing: 'var(--rd-tracking-label)',
          marginBottom: 2,
        }}
      >
        {data.type}
      </div>

      {/* Primary content */}
      <code
        style={{
          fontFamily: 'var(--rd-font-mono)',
          fontSize: 'var(--rd-text-sm)',
          color: 'var(--rd-text)',
          display: 'block',
        }}
      >
        {data.content.primary}
      </code>

      {/* Secondary */}
      {data.content.secondary && (
        <div
          style={{
            fontFamily: 'var(--rd-font-mono)',
            fontSize: 'var(--rd-text-xs)',
            color: 'var(--rd-text-dim)',
            marginTop: 2,
          }}
        >
          {data.content.secondary}
        </div>
      )}

      {/* Tertiary */}
      {data.content.tertiary && (
        <div
          style={{
            fontFamily: 'var(--rd-font-mono)',
            fontSize: 'var(--rd-text-xs)',
            color: 'var(--rd-text-ghost)',
            marginTop: 1,
          }}
        >
          {data.content.tertiary}
        </div>
      )}
    </div>
  );
}
```

### 9.8.2 Bus event type additions

Add these to `BusEvents` in `apps/explorer/src/data/types.ts`:

```typescript
// Tooltip events (emitted by scenes, consumed by Tooltip panel)
'tooltip:show': {
  type: 'block' | 'transaction' | 'address';
  x: number;
  y: number;
  content: {
    primary: string;
    secondary?: string;
    tertiary?: string;
  };
};
'tooltip:hide': undefined;
```

### 9.8.3 Scene integration example

Scenes emit tooltip events from their hover handlers. This example shows how
the constellation scene might emit tooltip data when hovering an address node:

```typescript
// Inside constellation scene hover handler:
canvas.addEventListener('mousemove', (e) => {
  const hoveredNode = findNearestNode(e.clientX, e.clientY);

  if (hoveredNode) {
    bus.emit('tooltip:show', {
      type: 'address',
      x: e.clientX,
      y: e.clientY,
      content: {
        primary: truncateHash(hoveredNode.address, 6),
        secondary: `${formatEth(hoveredNode.balance)} ETH`,
        tertiary: `${hoveredNode.txCount} txns`,
      },
    });
  } else {
    bus.emit('tooltip:hide', undefined);
  }
});
```

### 9.8.4 Verification

- [ ] Tooltip follows mouse with offset (12px right, -8px up)
- [ ] Shows entity summary: block (number+hash), tx (hash+value), address (addr+balance)
- [ ] Glass styling with small font (text-xs, text-sm)
- [ ] Disappears on mouse move away (with 80ms grace period to prevent flicker)
- [ ] Position clamped to viewport edges (16px padding)
- [ ] Left accent border color varies by type (rose=block, bone=tx, dream=address)
- [ ] FadeUp animation: opacity 0->1, translateY 4px->0, 200ms expo ease
- [ ] pointer-events: none (does not intercept clicks)

---

## 9.9 Hash Art Generator

Generates a deterministic visual pattern from a block hash. Used in BlockDetail,
terrain tile markers, and search results.

### 9.9.1 `apps/explorer/src/panels/HashArt.tsx`

React wrapper component:

```tsx
import { useEffect, useRef, memo } from 'react';
import { generateHashArt } from '@/utils/hashArt';

interface HashArtProps {
  hash: `0x${string}`;
  size?: number; // canvas size in px (default 64)
  className?: string;
  onClick?: () => void;
}

/**
 * Renders a deterministic generative art canvas from a block hash.
 * Memoized -- only re-renders when hash changes.
 */
export const HashArt = memo(function HashArt({
  hash,
  size = 64,
  className,
  onClick,
}: HashArtProps) {
  const canvasRef = useRef<HTMLCanvasElement>(null);

  useEffect(() => {
    const canvas = canvasRef.current;
    if (!canvas) return;

    const ctx = canvas.getContext('2d');
    if (!ctx) return;

    generateHashArt(ctx, hash, size);
  }, [hash, size]);

  return (
    <canvas
      ref={canvasRef}
      width={size}
      height={size}
      className={className}
      onClick={onClick}
      style={{
        width: size,
        height: size,
        imageRendering: 'pixelated',
        cursor: onClick ? 'pointer' : 'default',
      }}
      aria-label={`Generative art for hash ${hash}`}
      role="img"
    />
  );
});
```

### 9.9.2 `apps/explorer/src/utils/hashArt.ts`

The core algorithm: converts a 32-byte hash into an 8x8 mirrored grid with
a color palette derived from the hash bytes.

```typescript
/**
 * Hash Art Generator
 *
 * Converts a 0x-prefixed hex hash (64 hex chars) into a deterministic
 * visual pattern on a canvas.
 *
 * Algorithm:
 * 1. Parse hash into 32 bytes.
 * 2. Bytes 0-2: derive 3-color palette (hue rotation within ROSEDUST spectrum).
 * 3. Byte 3: select pattern algorithm (8 possible patterns via modulo).
 * 4. Bytes 4-19: fill an 8x4 half-grid (4 bits per byte, 16 bytes = 64 cells / 2).
 * 5. Mirror horizontally for bilateral symmetry.
 * 6. Byte 20: density threshold -- cells above threshold are filled, below are void.
 * 7. Render each filled cell as a colored rectangle on the canvas.
 *
 * The result is a unique, mirrored, deterministic image for every block hash.
 */

// ROSEDUST hue anchors (HSL hue values for palette generation)
const HUE_ROSE = 340;
const HUE_BONE = 40;
const HUE_DREAM = 240;
const HUE_ANCHORS = [HUE_ROSE, HUE_BONE, HUE_DREAM];

// ---------------------------------------------------------------------------
// Byte parsing
// ---------------------------------------------------------------------------

function hashToBytes(hash: `0x${string}`): Uint8Array {
  const hex = hash.slice(2); // strip 0x
  const bytes = new Uint8Array(32);
  for (let i = 0; i < 32; i++) {
    bytes[i] = parseInt(hex.slice(i * 2, i * 2 + 2), 16);
  }
  return bytes;
}

// ---------------------------------------------------------------------------
// Color palette
// ---------------------------------------------------------------------------

interface Palette {
  bg: string;
  colors: [string, string, string];
}

function derivePalette(bytes: Uint8Array): Palette {
  // Byte 0: base hue offset (shifts all 3 anchors)
  const hueShift = (bytes[0] / 255) * 30 - 15; // -15 to +15

  // Byte 1: saturation variation
  const satBase = 30 + (bytes[1] / 255) * 30; // 30-60%

  // Byte 2: lightness variation
  const lightBase = 40 + (bytes[2] / 255) * 20; // 40-60%

  const colors = HUE_ANCHORS.map((hue) => {
    const h = (hue + hueShift + 360) % 360;
    return `hsl(${h}, ${satBase}%, ${lightBase}%)`;
  }) as [string, string, string];

  // Background is always near-void
  const bg = '#0a0a0e';

  return { bg, colors };
}

// ---------------------------------------------------------------------------
// Grid generation (8x8, mirrored)
// ---------------------------------------------------------------------------

type Grid = number[][]; // 8x8, values 0-2 (color index) or -1 (empty)

function generateGrid(bytes: Uint8Array): Grid {
  const grid: Grid = Array.from({ length: 8 }, () => Array(8).fill(-1));

  // Byte 3: density threshold (0-255, higher = sparser)
  const threshold = bytes[3];

  // Bytes 4-19: fill the left half (columns 0-3) of each row
  for (let row = 0; row < 8; row++) {
    for (let col = 0; col < 4; col++) {
      const byteIndex = 4 + row * 2 + Math.floor(col / 2);
      const nibble = col % 2 === 0
        ? (bytes[byteIndex] >> 4) & 0x0f
        : bytes[byteIndex] & 0x0f;

      // Scale nibble to 0-255 range for threshold comparison
      const value = nibble * 17; // 0-255

      if (value > threshold) {
        // Color index from the nibble (3 colors)
        grid[row][col] = nibble % 3;
        // Mirror: column 7-col
        grid[row][7 - col] = nibble % 3;
      }
    }
  }

  return grid;
}

// ---------------------------------------------------------------------------
// Render
// ---------------------------------------------------------------------------

export function generateHashArt(
  ctx: CanvasRenderingContext2D,
  hash: `0x${string}`,
  size: number,
): void {
  const bytes = hashToBytes(hash);
  const palette = derivePalette(bytes);
  const grid = generateGrid(bytes);

  const cellSize = size / 8;

  // Clear with background
  ctx.fillStyle = palette.bg;
  ctx.fillRect(0, 0, size, size);

  // Draw grid cells
  for (let row = 0; row < 8; row++) {
    for (let col = 0; col < 8; col++) {
      const colorIndex = grid[row][col];
      if (colorIndex === -1) continue; // empty cell

      ctx.fillStyle = palette.colors[colorIndex];
      ctx.fillRect(
        col * cellSize,
        row * cellSize,
        cellSize,
        cellSize,
      );
    }
  }
}
```

### 9.9.3 Unit test: `apps/explorer/src/utils/hashArt.test.ts`

```typescript
import { describe, it, expect } from 'vitest';
import { generateHashArt } from './hashArt';

// Minimal mock canvas context for testing
function createMockContext() {
  const fills: { style: string; x: number; y: number; w: number; h: number }[] = [];

  return {
    fillStyle: '',
    fillRect(x: number, y: number, w: number, h: number) {
      fills.push({ style: this.fillStyle, x, y, w, h });
    },
    fills,
  } as unknown as CanvasRenderingContext2D & { fills: typeof fills };
}

describe('generateHashArt', () => {
  it('produces deterministic output for the same hash', () => {
    const hash = '0x6d1fb175dce4b6795ca4dc32a19a0d6b6ab5c25b8e48ce49aaee6c28d5d5a0c1' as `0x${string}`;
    const ctx1 = createMockContext();
    const ctx2 = createMockContext();

    generateHashArt(ctx1, hash, 64);
    generateHashArt(ctx2, hash, 64);

    expect(ctx1.fills).toEqual(ctx2.fills);
  });

  it('produces different output for different hashes', () => {
    const hash1 = '0x6d1fb175dce4b6795ca4dc32a19a0d6b6ab5c25b8e48ce49aaee6c28d5d5a0c1' as `0x${string}`;
    const hash2 = '0x0000000000000000000000000000000000000000000000000000000000000001' as `0x${string}`;

    const ctx1 = createMockContext();
    const ctx2 = createMockContext();

    generateHashArt(ctx1, hash1, 64);
    generateHashArt(ctx2, hash2, 64);

    // Different hashes should produce different fill patterns
    expect(ctx1.fills).not.toEqual(ctx2.fills);
  });

  it('always renders exactly 8x8 grid cells max (plus background)', () => {
    const hash = '0xffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff' as `0x${string}`;
    const ctx = createMockContext();

    generateHashArt(ctx, hash, 64);

    // First fill is always the background rect
    expect(ctx.fills[0]).toEqual(expect.objectContaining({ x: 0, y: 0, w: 64, h: 64 }));

    // Remaining fills are grid cells, each 8x8 pixels
    const cellFills = ctx.fills.slice(1);
    for (const fill of cellFills) {
      expect(fill.w).toBe(8);
      expect(fill.h).toBe(8);
    }
  });

  it('has bilateral symmetry (mirrored left-right)', () => {
    const hash = '0x6d1fb175dce4b6795ca4dc32a19a0d6b6ab5c25b8e48ce49aaee6c28d5d5a0c1' as `0x${string}`;
    const ctx = createMockContext();

    generateHashArt(ctx, hash, 64);

    // Build a lookup of filled cells (excluding background)
    const cellFills = ctx.fills.slice(1);
    const cellMap = new Map<string, string>();
    for (const fill of cellFills) {
      cellMap.set(`${fill.x},${fill.y}`, fill.style);
    }

    // For each filled cell on the left half, check mirror exists on right
    for (const fill of cellFills) {
      const col = fill.x / 8;
      if (col >= 4) continue; // only check left half
      const mirrorX = (7 - col) * 8;
      const mirrorKey = `${mirrorX},${fill.y}`;
      expect(cellMap.has(mirrorKey)).toBe(true);
      expect(cellMap.get(mirrorKey)).toBe(fill.style);
    }
  });
});
```

### 9.9.4 Verification

- [ ] 8x8 grid, mirrored horizontally for bilateral symmetry
- [ ] Color palette derived from first 3 bytes of hash (rose/bone/dream spectrum)
- [ ] Density controlled by byte 3 (higher = sparser pattern)
- [ ] Deterministic: same hash always produces same image
- [ ] Renders to small canvas element (default 64x64, 128x128 in BlockDetail)
- [ ] `imageRendering: pixelated` for crisp pixel art look
- [ ] Memoized: only re-renders when hash prop changes
- [ ] Unit tests pass for determinism, different hashes, symmetry

---

## 9.10 Formatting Utilities

Complete formatting functions used across all panels.

### 9.10.1 `apps/explorer/src/utils/format.ts`

```typescript
/**
 * Formatting utilities for the Kora Explorer.
 *
 * All functions are pure, side-effect-free, and handle edge cases
 * (null, zero, negative, very large values).
 */

// ---------------------------------------------------------------------------
// Hash / address truncation
// ---------------------------------------------------------------------------

/**
 * Truncate a hex hash: "0xabcdef...012345"
 * @param hash - The full 0x-prefixed hex string
 * @param chars - Number of hex chars to show on each side (default 6)
 */
export function truncateHash(hash: string, chars = 6): string {
  if (!hash || hash.length <= chars * 2 + 4) return hash ?? '';
  return `${hash.slice(0, chars + 2)}...${hash.slice(-chars)}`;
}

// ---------------------------------------------------------------------------
// ETH formatting
// ---------------------------------------------------------------------------

/**
 * Format a bigint wei value to a human-readable ETH string.
 * @param wei - Value in wei
 * @param decimals - Max decimal places (default 4)
 */
export function formatEth(wei: bigint, decimals = 4): string {
  if (wei === 0n) return '0';

  const negative = wei < 0n;
  const absWei = negative ? -wei : wei;

  // Split into integer and fractional parts
  const ethWhole = absWei / 10n ** 18n;
  const ethFrac = absWei % 10n ** 18n;

  // Format fractional part with zero-padding
  const fracStr = ethFrac.toString().padStart(18, '0').slice(0, decimals);

  // Trim trailing zeros from fractional part
  const trimmed = fracStr.replace(/0+$/, '');

  const sign = negative ? '-' : '';
  if (!trimmed) {
    return `${sign}${formatNumber(ethWhole)}`;
  }
  return `${sign}${formatNumber(ethWhole)}.${trimmed}`;
}

// ---------------------------------------------------------------------------
// Gas formatting
// ---------------------------------------------------------------------------

/**
 * Format a gas value with thousands separators.
 * @param gas - Gas amount (bigint or number)
 */
export function formatGas(gas: bigint | number): string {
  return formatNumber(typeof gas === 'number' ? BigInt(gas) : gas);
}

// ---------------------------------------------------------------------------
// Timestamp formatting
// ---------------------------------------------------------------------------

/**
 * Format a Unix timestamp to relative or absolute time.
 * @param ts - Unix timestamp in seconds (bigint)
 */
export function formatTimestamp(ts: bigint): string {
  const now = BigInt(Math.floor(Date.now() / 1000));
  const diff = now - ts;

  // Relative time for recent timestamps
  if (diff < 60n) return `${diff}s ago`;
  if (diff < 3600n) return `${diff / 60n}m ago`;
  if (diff < 86400n) return `${diff / 3600n}h ago`;

  // Absolute time for older timestamps
  const date = new Date(Number(ts) * 1000);
  const month = date.toLocaleString('en-US', { month: 'short' });
  const day = date.getDate();
  const hours = date.getHours().toString().padStart(2, '0');
  const mins = date.getMinutes().toString().padStart(2, '0');

  return `${month} ${day}, ${hours}:${mins}`;
}

// ---------------------------------------------------------------------------
// Number formatting
// ---------------------------------------------------------------------------

/**
 * Format a number with thousands separators.
 * @param n - Number (bigint or number)
 */
export function formatNumber(n: bigint | number): string {
  const str = n.toString();

  // Handle negative numbers
  if (str.startsWith('-')) {
    return '-' + addThousandsSep(str.slice(1));
  }

  return addThousandsSep(str);
}

function addThousandsSep(s: string): string {
  const parts: string[] = [];
  let remaining = s;

  while (remaining.length > 3) {
    parts.unshift(remaining.slice(-3));
    remaining = remaining.slice(0, -3);
  }

  parts.unshift(remaining);
  return parts.join(',');
}
```

### 9.10.2 Unit test: `apps/explorer/src/utils/format.test.ts`

Replace the existing stub test file with comprehensive tests:

```typescript
import { describe, it, expect } from 'vitest';
import {
  truncateHash,
  formatEth,
  formatGas,
  formatTimestamp,
  formatNumber,
} from './format';

// ---------------------------------------------------------------------------
// truncateHash
// ---------------------------------------------------------------------------

describe('truncateHash', () => {
  it('truncates a 66-char tx hash with default chars=6', () => {
    const hash = '0xabcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789';
    expect(truncateHash(hash)).toBe('0xabcdef...456789');
  });

  it('truncates a 42-char address with chars=4', () => {
    const addr = '0x1234567890abcdef1234567890abcdef12345678';
    expect(truncateHash(addr, 4)).toBe('0x1234...5678');
  });

  it('returns short strings unchanged', () => {
    expect(truncateHash('0xabcd')).toBe('0xabcd');
  });

  it('returns empty string for empty input', () => {
    expect(truncateHash('')).toBe('');
  });

  it('handles null/undefined gracefully', () => {
    expect(truncateHash(null as unknown as string)).toBe('');
    expect(truncateHash(undefined as unknown as string)).toBe('');
  });
});

// ---------------------------------------------------------------------------
// formatEth
// ---------------------------------------------------------------------------

describe('formatEth', () => {
  it('formats 1 ETH', () => {
    expect(formatEth(1_000_000_000_000_000_000n)).toBe('1');
  });

  it('formats 0.5 ETH', () => {
    expect(formatEth(500_000_000_000_000_000n)).toBe('0.5');
  });

  it('formats 0 ETH', () => {
    expect(formatEth(0n)).toBe('0');
  });

  it('formats large value: 1,234.5678 ETH', () => {
    expect(formatEth(1_234_567_800_000_000_000_000n)).toBe('1,234.5678');
  });

  it('formats fractional with custom decimals', () => {
    expect(formatEth(100_000_000_000_000n, 6)).toBe('0.0001');
  });

  it('trims trailing zeros', () => {
    expect(formatEth(1_500_000_000_000_000_000n)).toBe('1.5');
  });

  it('formats very small amounts (wei)', () => {
    expect(formatEth(1n, 18)).toBe('0.000000000000000001');
  });
});

// ---------------------------------------------------------------------------
// formatGas
// ---------------------------------------------------------------------------

describe('formatGas', () => {
  it('formats 21000 gas', () => {
    expect(formatGas(21000n)).toBe('21,000');
  });

  it('formats 250M gas limit', () => {
    expect(formatGas(250_000_000n)).toBe('250,000,000');
  });

  it('formats 0 gas', () => {
    expect(formatGas(0n)).toBe('0');
  });

  it('accepts number input', () => {
    expect(formatGas(21000)).toBe('21,000');
  });
});

// ---------------------------------------------------------------------------
// formatTimestamp
// ---------------------------------------------------------------------------

describe('formatTimestamp', () => {
  it('formats recent timestamp as relative', () => {
    const now = BigInt(Math.floor(Date.now() / 1000));
    expect(formatTimestamp(now - 5n)).toBe('5s ago');
  });

  it('formats minutes ago', () => {
    const now = BigInt(Math.floor(Date.now() / 1000));
    expect(formatTimestamp(now - 120n)).toBe('2m ago');
  });

  it('formats hours ago', () => {
    const now = BigInt(Math.floor(Date.now() / 1000));
    expect(formatTimestamp(now - 7200n)).toBe('2h ago');
  });

  it('formats old timestamp as absolute date', () => {
    // Jan 1, 2025 12:00 UTC
    const ts = BigInt(1735732800);
    const result = formatTimestamp(ts);
    // Should contain month and time
    expect(result).toMatch(/\w+ \d+, \d{2}:\d{2}/);
  });
});

// ---------------------------------------------------------------------------
// formatNumber
// ---------------------------------------------------------------------------

describe('formatNumber', () => {
  it('formats with thousands separators', () => {
    expect(formatNumber(1234567n)).toBe('1,234,567');
  });

  it('formats small numbers without separator', () => {
    expect(formatNumber(42n)).toBe('42');
  });

  it('formats zero', () => {
    expect(formatNumber(0n)).toBe('0');
  });

  it('formats negative numbers', () => {
    expect(formatNumber(-1234567n)).toBe('-1,234,567');
  });

  it('accepts number input', () => {
    expect(formatNumber(1234567)).toBe('1,234,567');
  });
});
```

### 9.10.3 Verification

- [ ] `truncateHash("0xabcd...ef01", 6)` produces correct truncation
- [ ] `formatEth(1000000000000000000n)` returns "1"
- [ ] `formatGas(21000n)` returns "21,000"
- [ ] `formatTimestamp` returns "2s ago" for recent, "Jan 1, 12:00" for old
- [ ] `formatNumber(1234567n)` returns "1,234,567"
- [ ] All edge cases handled: zero, null, negative, very large values
- [ ] All unit tests pass

---

## 9.11 Panel Composition in App.tsx

All panels are composed in the root layout. They render on top of the active scene.

### 9.11.1 Updated `apps/explorer/src/App.tsx`

```tsx
import { BrowserRouter, Routes, Route } from 'react-router';
import { SceneManager } from '@/scenes/SceneManager';
import { StatusBar } from '@/panels/StatusBar';
import { SceneNav } from '@/panels/SceneNav';
import { SearchOverlay } from '@/panels/SearchOverlay';
import { Tooltip } from '@/panels/Tooltip';
import { BlockDetail } from '@/panels/BlockDetail';
import { TxDetail } from '@/panels/TxDetail';
import { AddressDetail } from '@/panels/AddressDetail';

export function App() {
  return (
    <BrowserRouter>
      {/* ── Scene layer (z-index: 1) ── */}
      <SceneManager />

      {/* ── Atmospheric layers (grain, vignette, scanlines, rose wash) ── */}
      <div className="grain" />
      <div className="roseWash" />

      {/* ── Panel layer (z-index: 100+) ── */}
      <SceneNav />
      <StatusBar />
      <Tooltip />
      <SearchOverlay />

      {/* ── Detail routes (z-index: 700) ── */}
      <Routes>
        <Route path="/block/:numberOrHash" element={<BlockDetail />} />
        <Route path="/tx/:hash" element={<TxDetail />} />
        <Route path="/address/:address" element={<AddressDetail />} />
      </Routes>
    </BrowserRouter>
  );
}
```

### 9.11.2 Z-index layer summary

| Layer | Z-index | Components |
|-------|---------|------------|
| Scene (WebGL canvas) | `--rd-z-scene` (1) | SceneManager |
| Rose wash | `--rd-z-panels - 1` (99) | .roseWash |
| Panels | `--rd-z-panels` (100) | StatusBar, SceneNav |
| Overlay | `--rd-z-overlay` (500) | Tooltip |
| Search | `--rd-z-search` (600) | SearchOverlay |
| Detail | `--rd-z-detail` (700) | BlockDetail, TxDetail, AddressDetail |
| Grain | `--rd-z-grain` (9997) | .grain |
| Vignette | `--rd-z-vignette` (9998) | body::before |
| Scanlines | `--rd-z-scanlines` (9999) | body::after |

---

## 9.12 Complete Verification Checklist

### Block Detail
- [ ] BlockDetail shows all fields when block selected
- [ ] Hash art canvas renders uniquely per block
- [ ] Transaction list is scrollable with click-to-navigate
- [ ] Prev/next block navigation works (arrows + keyboard)
- [ ] All hash fields are copyable with flash feedback

### Transaction Detail
- [ ] TxDetail shows receipt and logs
- [ ] Status badge shows success (green) or reverted (red)
- [ ] Flow diagram shows from -> to with value
- [ ] Clicking addresses navigates to AddressDetail
- [ ] Input data renders in hex with scrollable overflow
- [ ] Block number link navigates to BlockDetail

### Address Detail
- [ ] AddressDetail shows balance and type (EOA/Contract)
- [ ] Balance displayed in Fraunces italic hero style
- [ ] Type determined by code length check
- [ ] Contract info shows code size when applicable
- [ ] Recent transactions scanned from cached blocks

### Search
- [ ] Search parses block numbers, tx hashes, addresses correctly
- [ ] Cmd+K (or Ctrl+K) opens search overlay
- [ ] `/` key opens search (when not in input field)
- [ ] Escape closes search overlay
- [ ] Clicking backdrop closes overlay
- [ ] Cache matches appear instantly
- [ ] RPC fallback fires after 150ms debounce
- [ ] ArrowUp/ArrowDown navigate results, Enter confirms
- [ ] Recent searches persisted in localStorage (last 8)
- [ ] Invalid input shows hint message

### Status Bar
- [ ] StatusBar shows live block number (updates on each new block)
- [ ] Connection LED updates: green (connected), amber (reconnecting), red (disconnected)
- [ ] FPS counter visible in dev mode only
- [ ] Block time estimate shown as "Xs ago"

### Scene Navigation
- [ ] Scene tabs switch scenes on click
- [ ] Number keys 1-4 switch scenes (not captured in input fields)
- [ ] Active tab has rose underline accent

### Panels & Styling
- [ ] All panels use glass styling (backdrop blur, 7% white border, no border-radius)
- [ ] Active panels have left rose accent border
- [ ] Collapse/expand animates panel body height
- [ ] Close button and Escape key close panels
- [ ] Drag-to-resize works on right panels (min 280px, max 800px)

### Tooltip
- [ ] Tooltip follows mouse with offset
- [ ] Shows entity summary based on type
- [ ] Clamped to viewport edges
- [ ] Disappears on mouse away with 80ms grace period

### Keyboard Navigation
- [ ] All panels reachable via Tab
- [ ] Focus ring: 2px rose outline with 2px offset
- [ ] Escape closes the topmost panel
- [ ] Arrow keys navigate within lists and between blocks

### Accessibility
- [ ] All panels have `role="region"` with `aria-label`
- [ ] Search has `role="combobox"` with `aria-expanded`
- [ ] Status LED has `aria-live="polite"`
- [ ] Scene tabs use `role="tablist"` with `aria-selected`
- [ ] Hash values in `<code>` tags with `aria-label` for full values
- [ ] Reduced motion: all transitions respect `prefers-reduced-motion`

### Ambient Mode
- [ ] Panels fade to 20% opacity after 30s idle
- [ ] Any interaction restores panels to 100% (200ms fade-in)
- [ ] StatusBar and SceneNav respect ambient opacity
