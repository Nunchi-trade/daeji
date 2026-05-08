import { jsx as _jsx, jsxs as _jsxs } from "react/jsx-runtime";
import { useMemo } from 'react';
import { useChainStore, useConsensusStore } from '@/data/store';
// Arc geometry: 270 degrees starting from bottom-left (225deg), clockwise
const RADIUS = 11;
const CX = 14;
const CY = 14;
const SWEEP_DEG = 270;
const CIRCUMFERENCE = 2 * Math.PI * RADIUS;
const ARC_LENGTH = (SWEEP_DEG / 360) * CIRCUMFERENCE;
// Start angle: 225deg (bottom-left), in SVG coords (0 = right, clockwise)
// We rotate the SVG coordinate system so arc starts at bottom-left
const START_ANGLE_DEG = 135; // 135deg from top = bottom-left (SVG y-axis flipped)
function describeArc(startDeg, sweepDeg, r, cx, cy) {
    const start = ((startDeg - 90) * Math.PI) / 180;
    const end = ((startDeg - 90 + sweepDeg) * Math.PI) / 180;
    const x1 = cx + r * Math.cos(start);
    const y1 = cy + r * Math.sin(start);
    const x2 = cx + r * Math.cos(end);
    const y2 = cy + r * Math.sin(end);
    const largeArc = sweepDeg > 180 ? 1 : 0;
    return `M ${x1} ${y1} A ${r} ${r} 0 ${largeArc} 1 ${x2} ${y2}`;
}
export function NetworkHealth() {
    const connectionState = useChainStore((s) => s.connectionState);
    const blockOrder = useChainStore((s) => s.blockOrder);
    const blocks = useChainStore((s) => s.blocks);
    const nullificationRate = useConsensusStore((s) => s.nullificationRate);
    const score = useMemo(() => {
        // 1. Finalization rate (35%): based on nullification rate from consensus store
        const finalizationScore = (1 - nullificationRate) * 100;
        // 2. Connectivity (20%)
        const connectivityScore = connectionState === 'connected'
            ? 100
            : connectionState === 'reconnecting'
                ? 50
                : 0;
        // 3. Block time regularity (25%): last 20 blocks' arrival times
        const recentOrder = blockOrder.slice(0, 20);
        let blockTimeScore = 50; // default when not enough data
        if (recentOrder.length >= 2) {
            const arrivals = [];
            for (const num of recentOrder) {
                const b = blocks.get(num);
                if (b && b._arrivalTime)
                    arrivals.push(b._arrivalTime);
            }
            if (arrivals.length >= 2) {
                let totalDiff = 0;
                let count = 0;
                for (let i = 0; i < arrivals.length - 1; i++) {
                    const diff = Math.abs(arrivals[i] - arrivals[i + 1]);
                    totalDiff += diff;
                    count++;
                }
                const avgMs = totalDiff / count;
                const avgSec = avgMs / 1000;
                // 1-5s => 100, >10s => 0, linear between 5-10
                if (avgSec <= 5) {
                    blockTimeScore = 100;
                }
                else if (avgSec >= 10) {
                    blockTimeScore = 0;
                }
                else {
                    blockTimeScore = ((10 - avgSec) / 5) * 100;
                }
            }
        }
        // 4. Gas load (20%): avg _activityLevel (gasUsed/gasLimit) from last 20 blocks
        let gasScore = 75; // default when not enough data
        if (recentOrder.length > 0) {
            let totalRatio = 0;
            let count = 0;
            for (const num of recentOrder) {
                const b = blocks.get(num);
                if (b) {
                    totalRatio += b._activityLevel;
                    count++;
                }
            }
            if (count > 0) {
                const avgRatio = totalRatio / count;
                // <0.5 => 100, >0.95 => 0, linear between
                if (avgRatio <= 0.5) {
                    gasScore = 100;
                }
                else if (avgRatio >= 0.95) {
                    gasScore = 0;
                }
                else {
                    gasScore = ((0.95 - avgRatio) / 0.45) * 100;
                }
            }
        }
        const weighted = finalizationScore * 0.35 +
            connectivityScore * 0.2 +
            blockTimeScore * 0.25 +
            gasScore * 0.2;
        return Math.round(Math.max(0, Math.min(100, weighted)));
    }, [nullificationRate, connectionState, blockOrder, blocks]);
    const arcColor = score > 70 ? '#7a8a78' : score > 40 ? '#c89a68' : '#cc5555';
    const bgPath = describeArc(START_ANGLE_DEG, SWEEP_DEG, RADIUS, CX, CY);
    const fgDashArray = ARC_LENGTH;
    const fgDashOffset = ARC_LENGTH * (1 - score / 100);
    return (_jsxs("svg", { width: 28, height: 28, viewBox: "0 0 28 28", style: { display: 'block', flexShrink: 0 }, "aria-label": `Network health: ${score}`, children: [_jsx("path", { d: bgPath, fill: "none", stroke: "var(--rd-text-ghost, #3a303a)", strokeWidth: 3, strokeLinecap: "round" }), _jsx("path", { d: bgPath, fill: "none", stroke: arcColor, strokeWidth: 3, strokeLinecap: "round", strokeDasharray: fgDashArray, strokeDashoffset: fgDashOffset }), _jsx("text", { x: CX, y: CY + 3, textAnchor: "middle", fontFamily: "var(--rd-font-mono, monospace)", fontSize: 8, fill: "var(--rd-text-ghost, #3a303a)", style: { userSelect: 'none' }, children: score })] }));
}
