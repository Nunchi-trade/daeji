import { jsx as _jsx } from "react/jsx-runtime";
import { useMemo } from 'react';
import { useChainStore } from '@/data/store';
const VIEW_W = 320;
const VIEW_H = 48;
const COUNT = 20;
export function TxVolumeChart() {
    const blockOrder = useChainStore((s) => s.blockOrder);
    const blocks = useChainStore((s) => s.blocks);
    const bars = useMemo(() => {
        const slice = blockOrder.slice(0, COUNT).slice().reverse();
        if (slice.length === 0)
            return [];
        const values = slice.map((num) => {
            const block = blocks.get(num);
            return block ? block.transactions.length : 0;
        });
        const max = Math.max(...values);
        const barW = VIEW_W / slice.length - 1;
        return values.map((v, i) => {
            const h = max === 0 ? 1 : Math.max(1, (v / max) * VIEW_H);
            return {
                x: i * (barW + 1),
                y: VIEW_H - h,
                w: barW,
                h,
            };
        });
    }, [blockOrder, blocks]);
    if (bars.length === 0) {
        return (_jsx("div", { style: { padding: '8px 14px' }, children: _jsx("svg", { width: "100%", viewBox: `0 0 ${VIEW_W} ${VIEW_H}`, preserveAspectRatio: "none", style: { height: VIEW_H, display: 'block' }, "aria-label": "Transaction volume chart (no data)", children: _jsx("line", { x1: 0, y1: VIEW_H / 2, x2: VIEW_W, y2: VIEW_H / 2, stroke: "var(--rd-text-ghost)", strokeWidth: 1, strokeDasharray: "4 4" }) }) }));
    }
    return (_jsx("div", { style: { padding: '8px 14px' }, children: _jsx("svg", { width: "100%", viewBox: `0 0 ${VIEW_W} ${VIEW_H}`, preserveAspectRatio: "none", style: { height: VIEW_H, display: 'block' }, "aria-label": "Transaction volume chart", children: bars.map((bar, i) => (_jsx("rect", { x: bar.x, y: bar.y, width: bar.w, height: bar.h, fill: "var(--rd-bone)", rx: 2 }, i))) }) }));
}
