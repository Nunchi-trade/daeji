import { jsx as _jsx, jsxs as _jsxs } from "react/jsx-runtime";
import { useMemo } from 'react';
import { useChainStore } from '@/data/store';
import { toBezierPath } from './svg-utils';
export function BlockTimeSparkline({ width = 160, height = 48, }) {
    const blockOrder = useChainStore((s) => s.blockOrder);
    const blocks = useChainStore((s) => s.blocks);
    const { path, avg, refY } = useMemo(() => {
        const slice = blockOrder.slice(0, 20);
        if (slice.length < 2) {
            return { path: '', avg: 0, refY: height / 2 };
        }
        // Compute diffs between consecutive blocks (reversed for left-to-right)
        const reversed = [...slice].reverse();
        const diffs = [];
        for (let i = 0; i < reversed.length - 1; i++) {
            const curr = blocks.get(reversed[i]);
            const next = blocks.get(reversed[i + 1]);
            if (curr && next) {
                diffs.push((curr._arrivalTime - next._arrivalTime) / 1000);
            }
        }
        if (diffs.length === 0) {
            return { path: '', avg: 0, refY: height / 2 };
        }
        const average = diffs.reduce((a, b) => a + b, 0) / diffs.length;
        const pad = 4;
        const minVal = Math.min(...diffs);
        const maxVal = Math.max(...diffs);
        const range = maxVal - minVal || 1;
        const points = diffs.map((d, i) => ({
            x: (i / (diffs.length - 1)) * width,
            y: height - pad - ((d - minVal) / range) * (height - pad * 2),
        }));
        const avgNorm = height - pad - ((average - minVal) / range) * (height - pad * 2);
        return {
            path: toBezierPath(points),
            avg: average,
            refY: avgNorm,
        };
    }, [blockOrder, blocks, width, height]);
    const isEmpty = !path;
    return (_jsxs("svg", { width: width, height: height, viewBox: `0 0 ${width} ${height}`, style: { display: 'block', overflow: 'visible' }, children: [_jsx("line", { x1: 0, y1: isEmpty ? height / 2 : refY, x2: width, y2: isEmpty ? height / 2 : refY, stroke: "var(--rd-text-ghost)", strokeWidth: 1, strokeDasharray: "4 4" }), !isEmpty && (_jsx("path", { d: path, fill: "none", stroke: "var(--rd-dream)", strokeWidth: 1.5, strokeLinecap: "round", strokeLinejoin: "round" })), !isEmpty && avg > 0 && (_jsxs("text", { x: width, y: 10, textAnchor: "end", fill: "var(--rd-text-dim)", fontSize: 10, fontFamily: "monospace", style: { fontVariantNumeric: 'tabular-nums' }, children: [avg.toFixed(1), "s"] }))] }));
}
