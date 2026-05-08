import { jsx as _jsx, jsxs as _jsxs } from "react/jsx-runtime";
import { useState } from 'react';
import { useNavigate } from 'react-router';
import { AnimatePresence, motion } from 'framer-motion';
import { useChainStore } from '@/data/store';
import { truncateHash, formatGas } from '@/lib/format';
import { PanelHeader } from '@/panels/PanelHeader';
import { BlockHeatstrip } from '@/panels/charts/BlockHeatstrip';
function GasBar({ ratio }) {
    const pct = Math.min(100, Math.max(0, ratio * 100));
    const color = pct > 80 ? '#e05555' : pct > 50 ? '#e8a84c' : '#aa7088';
    return (_jsx("div", { style: {
            width: 100,
            height: 7,
            background: 'rgba(255,255,255,0.06)',
            borderRadius: 2,
            overflow: 'hidden',
            flexShrink: 0,
        }, children: _jsx("div", { style: {
                width: `${Math.max(2, pct)}%`,
                height: '100%',
                background: color,
                transition: 'width 300ms ease-out',
            } }) }));
}
function timeAgo(arrivalTime) {
    const elapsed = Math.max(0, Date.now() - arrivalTime);
    if (elapsed < 1000)
        return 'now';
    if (elapsed < 60_000)
        return `${Math.round(elapsed / 1000)}s ago`;
    return `${Math.round(elapsed / 60_000)}m ago`;
}
function accentColor(ratio) {
    if (ratio >= 0.7)
        return '#cc5555';
    if (ratio >= 0.3)
        return '#c89a68';
    return '#7a8a78';
}
function BlockRow({ num, block }) {
    const navigate = useNavigate();
    const [hovered, setHovered] = useState(false);
    const gasRatio = block.gasLimit > 0n
        ? Number(block.gasUsed) / Number(block.gasLimit)
        : 0;
    return (_jsxs(motion.div, { initial: { opacity: 0, x: -20 }, animate: { opacity: 1, x: 0 }, exit: { opacity: 0, x: -20 }, transition: { duration: 0.3, ease: [0.16, 1, 0.3, 1] }, style: {
            position: 'relative',
            display: 'grid',
            gridTemplateColumns: '1fr auto',
            gap: '3px 16px',
            padding: '10px 14px 10px 17px',
            borderBottom: '1px solid rgba(255,255,255,0.04)',
            fontFamily: 'var(--rd-font-mono)',
            cursor: 'pointer',
            background: hovered ? 'var(--rd-glass-bg-hover)' : 'transparent',
            transition: 'background 80ms ease-out',
        }, onMouseEnter: () => setHovered(true), onMouseLeave: () => setHovered(false), onClick: () => navigate('/block/' + Number(block.number)), children: [_jsx("div", { style: {
                    position: 'absolute',
                    left: 0,
                    top: 0,
                    bottom: 0,
                    width: 3,
                    background: accentColor(gasRatio),
                } }), _jsxs("div", { style: { display: 'flex', alignItems: 'center', gap: 10 }, children: [_jsxs("span", { style: {
                            color: '#dca5bd',
                            fontVariantNumeric: 'tabular-nums',
                            fontSize: '14px',
                            fontWeight: 600,
                        }, children: ["#", Number(block.number).toLocaleString()] }), _jsx("span", { style: { color: '#6a5a62', fontSize: '12px' }, children: truncateHash(block.hash, 5) })] }), _jsx("span", { style: {
                    color: '#8a7a82',
                    textAlign: 'right',
                    fontVariantNumeric: 'tabular-nums',
                    fontSize: '12px',
                }, children: timeAgo(block._arrivalTime) }), _jsxs("div", { style: {
                    display: 'flex',
                    alignItems: 'center',
                    gap: 10,
                    color: '#9a8a92',
                    fontSize: '12px',
                }, children: [_jsxs("span", { style: { fontWeight: 500 }, children: [block.transactions.length, " txs"] }), _jsx(GasBar, { ratio: gasRatio }), _jsxs("span", { style: { fontVariantNumeric: 'tabular-nums', color: '#7a6a72' }, children: [formatGas(block.gasUsed), " gas"] })] }), _jsxs("span", { style: {
                    color: gasRatio > 0.5 ? '#e8a84c' : '#6a5a62',
                    textAlign: 'right',
                    fontVariantNumeric: 'tabular-nums',
                    fontSize: '11px',
                }, children: [(gasRatio * 100).toFixed(0), "%"] })] }, num.toString()));
}
export function BlockFeed() {
    const blockOrder = useChainStore((s) => s.blockOrder);
    const blocks = useChainStore((s) => s.blocks);
    const visibleBlocks = blockOrder.slice(0, 40);
    return (_jsxs("div", { style: { display: 'flex', flexDirection: 'column', overflow: 'hidden' }, children: [_jsx(PanelHeader, { icon: "blocks", title: "Latest Blocks" }), _jsx(BlockHeatstrip, {}), _jsxs("div", { style: {
                    overflowY: 'auto',
                    flex: 1,
                    scrollbarWidth: 'thin',
                    scrollbarColor: 'rgba(170,112,136,0.3) transparent',
                }, children: [visibleBlocks.length === 0 && (_jsx("div", { style: {
                            padding: '32px 14px',
                            fontFamily: 'var(--rd-font-mono)',
                            fontSize: '13px',
                            color: '#6a5a62',
                            textAlign: 'center',
                        }, children: "Waiting for blocks..." })), _jsx(AnimatePresence, { initial: false, children: visibleBlocks.map((num) => {
                            const block = blocks.get(num);
                            if (!block)
                                return null;
                            return _jsx(BlockRow, { num: num, block: block }, num.toString());
                        }) })] })] }));
}
