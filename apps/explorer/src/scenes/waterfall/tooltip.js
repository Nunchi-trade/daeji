import { jsxs as _jsxs } from "react/jsx-runtime";
import { useState, useEffect } from 'react';
export function WaterfallTooltip({ setRef }) {
    const [data, setData] = useState(null);
    useEffect(() => {
        setRef.current = setData;
    }, [setRef]);
    if (!data)
        return null;
    const { block, x, y } = data;
    const gasRatio = Number(block.gasUsed) / Number(block.gasLimit);
    const age = Date.now() - block._arrivalTime;
    const ageSec = (age / 1000).toFixed(1);
    return (_jsxs("div", { style: {
            position: 'fixed',
            left: x + 12,
            top: y - 60,
            background: 'rgba(8, 8, 12, 0.85)',
            backdropFilter: 'blur(12px)',
            border: '1px solid rgba(255, 255, 255, 0.07)',
            padding: '8px 12px',
            fontFamily: '"JetBrains Mono", monospace',
            fontSize: '10px',
            color: '#c8b8c0',
            pointerEvents: 'none',
            zIndex: 200,
            lineHeight: 1.6,
            whiteSpace: 'nowrap',
        }, children: [_jsxs("div", { style: { color: '#dca5bd', marginBottom: 2 }, children: ["BLOCK ", Number(block.number).toLocaleString()] }), _jsxs("div", { style: { color: '#6a5a68', letterSpacing: '0.06em' }, children: [block.hash.slice(0, 18), "...", block.hash.slice(-8)] }), _jsxs("div", { children: ["GAS ", (gasRatio * 100).toFixed(1), "%", _jsxs("span", { style: { color: '#6a5a68' }, children: [' ', "(", formatGasCompact(Number(block.gasUsed)), " / ", formatGasCompact(Number(block.gasLimit)), ")"] })] }), _jsxs("div", { children: [block.transactions.length, " TXN", _jsxs("span", { style: { color: '#6a5a68' }, children: [" ", ageSec, "s ago"] })] })] }));
}
function formatGasCompact(gas) {
    if (gas >= 1e9)
        return `${(gas / 1e9).toFixed(1)}B`;
    if (gas >= 1e6)
        return `${(gas / 1e6).toFixed(1)}M`;
    if (gas >= 1e3)
        return `${(gas / 1e3).toFixed(0)}K`;
    return gas.toString();
}
