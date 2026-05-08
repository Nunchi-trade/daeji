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
  resizable?: boolean;
  ariaLabel?: string;
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

  // Keyboard: Escape to close
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

  // Drag to resize
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

  const resolvedWidth = panelWidth ? `${panelWidth}px` : width;

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
        {/* Header */}
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
            {title && <span className={styles.label}>{title}</span>}

            <span style={{ flex: 1 }} />

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

        {/* Body (collapsible) */}
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

        {/* Resize handle */}
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
