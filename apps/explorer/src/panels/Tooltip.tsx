import { useState, useEffect, useRef, useCallback } from 'react';
import { bus } from '@/data/bus';
import styles from '@/design/glass.module.css';

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

interface TooltipData {
  type: 'block' | 'transaction' | 'address';
  x: number;
  y: number;
  content: {
    primary: string;
    secondary?: string;
    tertiary?: string;
  };
}

// Viewport edge clamping constants
const TOOLTIP_OFFSET_X = 12;
const TOOLTIP_OFFSET_Y = -8;
const TOOLTIP_MAX_WIDTH = 240;
const VIEWPORT_PADDING = 16;

// ---------------------------------------------------------------------------
// Component
// ---------------------------------------------------------------------------

export function Tooltip() {
  const [data, setData] = useState<TooltipData | null>(null);
  const [visible, setVisible] = useState(false);
  const tooltipRef = useRef<HTMLDivElement>(null);
  const hideTimerRef = useRef<ReturnType<typeof setTimeout> | undefined>(undefined);

  // Listen to bus events
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

  // Position clamping
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

  // Color by type
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
      {/* Type label */}
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
