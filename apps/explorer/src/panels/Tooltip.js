import { jsx as _jsx, jsxs as _jsxs } from "react/jsx-runtime";
import { useState, useEffect, useRef, useCallback } from 'react';
import { bus } from '@/data/bus';
import styles from '@/design/glass.module.css';
// Viewport edge clamping constants
const TOOLTIP_OFFSET_X = 12;
const TOOLTIP_OFFSET_Y = -8;
const TOOLTIP_MAX_WIDTH = 240;
const VIEWPORT_PADDING = 16;
// ---------------------------------------------------------------------------
// Component
// ---------------------------------------------------------------------------
export function Tooltip() {
    const [data, setData] = useState(null);
    const [visible, setVisible] = useState(false);
    const tooltipRef = useRef(null);
    const hideTimerRef = useRef(undefined);
    // Listen to bus events
    useEffect(() => {
        const showHandler = (payload) => {
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
        bus.on('tooltip:show', showHandler);
        bus.on('tooltip:hide', hideHandler);
        return () => {
            bus.off('tooltip:show', showHandler);
            bus.off('tooltip:hide', hideHandler);
            clearTimeout(hideTimerRef.current);
        };
    }, []);
    // Position clamping
    const getClampedPosition = useCallback(() => {
        if (!data)
            return { display: 'none' };
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
    if (!visible || !data)
        return null;
    // Color by type
    const accentColor = data.type === 'block'
        ? 'var(--rd-rose)'
        : data.type === 'transaction'
            ? 'var(--rd-bone-bright)'
            : 'var(--rd-dream)';
    return (_jsxs("div", { ref: tooltipRef, className: styles.panel, style: {
            position: 'fixed',
            ...getClampedPosition(),
            maxWidth: TOOLTIP_MAX_WIDTH,
            padding: 'var(--rd-space-sm) var(--rd-space-md)',
            zIndex: 'var(--rd-z-overlay)',
            pointerEvents: 'none',
            opacity: visible ? 1 : 0,
            transform: visible ? 'translateY(0)' : 'translateY(4px)',
            transition: `
          opacity var(--rd-duration-normal) var(--rd-ease-out),
          transform var(--rd-duration-normal) var(--rd-ease-out)
        `,
            borderLeft: `2px solid ${accentColor}`,
        }, children: [_jsx("div", { style: {
                    fontFamily: 'var(--rd-font-mono)',
                    fontSize: 'var(--rd-text-xs)',
                    color: accentColor,
                    textTransform: 'uppercase',
                    letterSpacing: 'var(--rd-tracking-label)',
                    marginBottom: 2,
                }, children: data.type }), _jsx("code", { style: {
                    fontFamily: 'var(--rd-font-mono)',
                    fontSize: 'var(--rd-text-sm)',
                    color: 'var(--rd-text)',
                    display: 'block',
                }, children: data.content.primary }), data.content.secondary && (_jsx("div", { style: {
                    fontFamily: 'var(--rd-font-mono)',
                    fontSize: 'var(--rd-text-xs)',
                    color: 'var(--rd-text-dim)',
                    marginTop: 2,
                }, children: data.content.secondary })), data.content.tertiary && (_jsx("div", { style: {
                    fontFamily: 'var(--rd-font-mono)',
                    fontSize: 'var(--rd-text-xs)',
                    color: 'var(--rd-text-ghost)',
                    marginTop: 1,
                }, children: data.content.tertiary }))] }));
}
