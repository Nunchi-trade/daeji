import { jsx as _jsx, jsxs as _jsxs, Fragment as _Fragment } from "react/jsx-runtime";
import { useMemo } from 'react';
import { useChainStore } from '@/data/store';
import { toBezierPath } from './charts/svg-utils';
// ---------------------------------------------------------------------------
// Component
// ---------------------------------------------------------------------------
export function FeeWaveform({ width = 200, height = 32, color = 'var(--rd-rose)', showLabels = false, }) {
    const feeHistory = useChainStore((s) => s.feeHistory);
    const { path, fillPath, minLabel, maxLabel } = useMemo(() => {
        if (feeHistory.length < 2) {
            return { path: '', fillPath: '', minLabel: '', maxLabel: '' };
        }
        // Convert baseFee bigints to numbers (in gwei)
        const values = feeHistory.map((f) => Number(f.baseFee) / 1e9);
        const min = Math.min(...values);
        const max = Math.max(...values);
        const range = max - min || 1;
        const padY = 2; // vertical padding
        const usableHeight = height - padY * 2;
        const points = values.map((v, i) => ({
            x: (i / (values.length - 1)) * width,
            y: padY + usableHeight - ((v - min) / range) * usableHeight,
        }));
        const linePath = toBezierPath(points);
        // Fill path: close to bottom
        const fill = linePath +
            ` L ${points[points.length - 1].x} ${height}` +
            ` L ${points[0].x} ${height} Z`;
        return {
            path: linePath,
            fillPath: fill,
            minLabel: `${min.toFixed(2)}`,
            maxLabel: `${max.toFixed(2)}`,
        };
    }, [feeHistory, width, height]);
    if (feeHistory.length < 2) {
        return (_jsx("svg", { width: width, height: height, style: { display: 'block' }, "aria-label": "Fee waveform (no data)", children: _jsx("line", { x1: 0, y1: height / 2, x2: width, y2: height / 2, stroke: "var(--rd-text-ghost)", strokeWidth: 1, strokeDasharray: "4 4" }) }));
    }
    return (_jsxs("svg", { width: width, height: height, style: { display: 'block' }, "aria-label": "Base fee waveform", children: [_jsx("defs", { children: _jsxs("linearGradient", { id: "feeGrad", x1: "0", y1: "0", x2: "0", y2: "1", children: [_jsx("stop", { offset: "0%", stopColor: color, stopOpacity: 0.2 }), _jsx("stop", { offset: "100%", stopColor: color, stopOpacity: 0 })] }) }), _jsx("path", { d: fillPath, fill: "url(#feeGrad)" }), _jsx("path", { d: path, fill: "none", stroke: color, strokeWidth: 1.5, strokeLinecap: "round", strokeLinejoin: "round" }), showLabels && (_jsxs(_Fragment, { children: [_jsx("text", { x: width - 2, y: 8, textAnchor: "end", fill: "var(--rd-text-dim)", fontSize: 8, fontFamily: "var(--rd-font-mono)", children: maxLabel }), _jsx("text", { x: width - 2, y: height - 2, textAnchor: "end", fill: "var(--rd-text-dim)", fontSize: 8, fontFamily: "var(--rd-font-mono)", children: minLabel })] }))] }));
}
