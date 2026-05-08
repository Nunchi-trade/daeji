import { jsx as _jsx, jsxs as _jsxs } from "react/jsx-runtime";
import { useMemo, useState } from 'react';
import { useConsensusStore } from '@/data/store';
// ================================================================
// CONSTANTS
// ================================================================
const BAR_COUNT = 32;
const BAR_WIDTH = 12;
const BAR_GAP = 3;
const MAX_BAR_HEIGHT = 60;
const CHART_HEIGHT = 80;
const COLORS = {
    finalized: '#aa7088', // --rd-rose
    nullified: '#3a303a', // --rd-text-ghost
    violation: '#cc5555', // --rd-danger
};
// ================================================================
// COMPONENT
// ================================================================
export function RoundHistory() {
    const roundHistory = useConsensusStore((s) => s.roundHistory);
    const [hoveredIndex, setHoveredIndex] = useState(null);
    const records = useMemo(() => {
        return roundHistory.slice(-BAR_COUNT).map((round) => ({
            view: round.view,
            durationMs: round.finalizationAt
                ? round.finalizationAt - round.startedAt
                : round.nullificationAt
                    ? round.nullificationAt - round.startedAt
                    : 0,
            outcome: round.phase === 'finalized'
                ? 'finalized'
                : round.phase === 'nullified'
                    ? 'nullified'
                    : 'finalized',
        }));
    }, [roundHistory]);
    const maxDuration = Math.max(500, ...records.map((r) => r.durationMs));
    const nullifiedCount = records.filter((r) => r.outcome === 'nullified').length;
    const avgMs = records.length > 0
        ? Math.round(records.reduce((s, r) => s + r.durationMs, 0) / records.length)
        : 0;
    const violationCount = records.filter((r) => r.outcome === 'violation').length;
    return (_jsxs("div", { style: {
            width: '100%',
            height: `${CHART_HEIGHT + 30}px`,
            background: 'rgba(8, 8, 12, 0.45)',
            borderTop: '1px solid rgba(255, 255, 255, 0.04)',
            padding: '8px 16px',
            fontFamily: '"JetBrains Mono", monospace',
            position: 'relative',
        }, children: [_jsx("div", { style: {
                    fontSize: '10px',
                    letterSpacing: '0.28em',
                    color: '#6a5a68',
                    textTransform: 'uppercase',
                    marginBottom: 4,
                }, children: "ROUND HISTORY" }), _jsx("div", { style: {
                    display: 'flex',
                    alignItems: 'flex-end',
                    gap: `${BAR_GAP}px`,
                    height: `${MAX_BAR_HEIGHT}px`,
                }, children: records.map((record, i) => {
                    const barHeight = Math.max(2, (record.durationMs / maxDuration) * MAX_BAR_HEIGHT);
                    const isHovered = hoveredIndex === i;
                    return (_jsx("div", { onMouseEnter: () => setHoveredIndex(i), onMouseLeave: () => setHoveredIndex(null), style: {
                            width: `${BAR_WIDTH}px`,
                            height: `${barHeight}px`,
                            background: COLORS[record.outcome],
                            opacity: isHovered ? 1 : 0.7,
                            transition: 'opacity 80ms ease-out',
                            cursor: 'pointer',
                            position: 'relative',
                        }, title: `View ${record.view}: ${record.durationMs}ms (${record.outcome})` }, record.view));
                }) }), hoveredIndex !== null && records[hoveredIndex] && (_jsxs("div", { style: {
                    position: 'absolute',
                    bottom: `${CHART_HEIGHT + 8}px`,
                    left: `${16 + hoveredIndex * (BAR_WIDTH + BAR_GAP)}px`,
                    background: 'rgba(8, 8, 12, 0.9)',
                    border: '1px solid rgba(255, 255, 255, 0.07)',
                    padding: '4px 8px',
                    fontSize: '9px',
                    color: '#c8b8c0',
                    whiteSpace: 'nowrap',
                    pointerEvents: 'none',
                    zIndex: 10,
                }, children: [_jsxs("div", { children: ["View ", records[hoveredIndex].view] }), _jsxs("div", { children: [records[hoveredIndex].durationMs, "ms"] }), _jsx("div", { style: { color: COLORS[records[hoveredIndex].outcome] }, children: records[hoveredIndex].outcome })] })), _jsxs("div", { style: {
                    fontSize: '9px',
                    color: '#6a5a68',
                    marginTop: 4,
                    display: 'flex',
                    gap: 16,
                }, children: [_jsxs("span", { children: ["avg: ", avgMs, "ms"] }), _jsxs("span", { children: ["nullified: ", nullifiedCount, "/", records.length, records.length > 0 &&
                                ` (${Math.round((nullifiedCount / records.length) * 100)}%)`] }), _jsxs("span", { style: { color: violationCount > 0 ? '#cc5555' : undefined }, children: ["violations: ", violationCount] })] })] }));
}
