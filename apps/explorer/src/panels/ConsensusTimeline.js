import { jsx as _jsx, jsxs as _jsxs } from "react/jsx-runtime";
import { useMemo } from 'react';
import { useConsensusStore } from '@/data/store';
const MAX_DOTS = 24;
const SVG_WIDTH = 320;
const SVG_HEIGHT = 40;
const DOT_Y = 20;
const DOT_R = 4;
const PAD_X = 12;
function dotColor(phase) {
    if (phase === 'finalized')
        return 'var(--rd-success)';
    if (phase === 'nullified')
        return 'var(--rd-warning)';
    if (phase === 'proposing' ||
        phase === 'notarizing' ||
        phase === 'certifying' ||
        phase === 'finalizing') {
        return 'var(--rd-rose)';
    }
    return 'var(--rd-text-ghost)';
}
function isActive(phase) {
    return (phase === 'proposing' ||
        phase === 'notarizing' ||
        phase === 'certifying' ||
        phase === 'finalizing');
}
export function ConsensusTimeline() {
    const roundHistory = useConsensusStore((s) => s.roundHistory);
    const currentRound = useConsensusStore((s) => s.currentRound);
    const dots = useMemo(() => {
        const rounds = currentRound
            ? [currentRound, ...roundHistory]
            : [...roundHistory];
        const visible = rounds.slice(0, MAX_DOTS).reverse();
        const count = visible.length;
        if (count === 0)
            return [];
        const usableWidth = SVG_WIDTH - PAD_X * 2;
        const step = count > 1 ? usableWidth / (count - 1) : 0;
        return visible.map((round, i) => ({
            x: count === 1 ? SVG_WIDTH / 2 : PAD_X + i * step,
            view: round.view,
            phase: round.phase,
            color: dotColor(round.phase),
            active: isActive(round.phase),
            showLabel: count <= 1 || i % 4 === 0,
        }));
    }, [roundHistory, currentRound]);
    const hasDots = dots.length > 0;
    return (_jsxs("div", { children: [_jsx("div", { style: {
                    padding: '10px 14px',
                    fontFamily: 'var(--rd-font-mono)',
                    fontSize: '12px',
                    fontWeight: 600,
                    textTransform: 'uppercase',
                    letterSpacing: '0.1em',
                    color: '#aa7088',
                    borderBottom: '1px solid rgba(255,255,255,0.06)',
                }, children: "Consensus Timeline" }), _jsx("div", { style: { padding: '8px 14px' }, children: !hasDots ? (_jsx("div", { style: {
                        fontFamily: 'var(--rd-font-mono)',
                        fontSize: '11px',
                        color: 'var(--rd-text-ghost)',
                        textAlign: 'center',
                        padding: '8px 0',
                    }, children: "No consensus data" })) : (_jsxs("svg", { width: "100%", viewBox: `0 0 ${SVG_WIDTH} ${SVG_HEIGHT}`, preserveAspectRatio: "xMidYMid meet", children: [_jsx("style", { children: `
              @keyframes rd-pulse {
                0%, 100% { opacity: 1; }
                50% { opacity: 0.4; }
              }
              .rd-dot-active {
                animation: rd-pulse 1.4s ease-in-out infinite;
              }
            ` }), dots.length > 1 && (_jsx("line", { x1: dots[0].x, y1: DOT_Y, x2: dots[dots.length - 1].x, y2: DOT_Y, stroke: "var(--rd-text-ghost)", strokeWidth: 1 })), dots.map((d) => (_jsx("circle", { cx: d.x, cy: DOT_Y, r: DOT_R, fill: d.color, className: d.active ? 'rd-dot-active' : undefined }, d.view))), dots
                            .filter((d) => d.showLabel)
                            .map((d) => (_jsx("text", { x: d.x, y: 34, textAnchor: "middle", fontFamily: "var(--rd-font-mono)", fontSize: 7, fill: "var(--rd-text-dim)", children: d.view }, `label-${d.view}`)))] })) })] }));
}
