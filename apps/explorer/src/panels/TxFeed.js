import { jsx as _jsx, jsxs as _jsxs } from "react/jsx-runtime";
import { AnimatePresence, motion } from 'framer-motion';
import { useChainStore } from '@/data/store';
import { truncateAddress, formatEth } from '@/lib/format';
import { PanelHeader } from '@/panels/PanelHeader';
const METHOD_NAMES = {
    '0xa9059cbb': 'transfer',
    '0x23b872dd': 'transferFrom',
    '0x095ea7b3': 'approve',
    '0x40c10f19': 'mint',
    '0x42966c68': 'burn',
    '0x3593564c': 'execute',
    '0x38ed1739': 'swap',
    '0x5ae401dc': 'multicall',
};
function getMethodName(input) {
    if (!input || input === '0x' || input.length < 10)
        return null;
    const selector = input.slice(0, 10).toLowerCase();
    return METHOD_NAMES[selector] ?? null;
}
function getDotColor(value) {
    if (value === 0n)
        return { color: 'var(--rd-text-ghost, #3a303a)' };
    const eth = Number(value) / 1e18;
    if (eth < 0.01)
        return { color: 'var(--rd-dream, #7a7a98)' };
    if (eth < 1)
        return { color: 'var(--rd-bone, #b8a880)' };
    return { color: 'var(--rd-bone-bright, #d8c8a0)', glow: true };
}
export function TxFeed() {
    const blockOrder = useChainStore((s) => s.blockOrder);
    const blocks = useChainStore((s) => s.blocks);
    const recentTxs = [];
    for (const num of blockOrder.slice(0, 50)) {
        const b = blocks.get(num);
        if (!b)
            continue;
        for (const tx of b.transactions) {
            recentTxs.push({
                hash: tx.hash,
                from: tx.from,
                to: tx.to,
                value: tx.value,
                blockNumber: b.number,
                input: tx.input,
            });
            if (recentTxs.length >= 25)
                break;
        }
        if (recentTxs.length >= 25)
            break;
    }
    return (_jsxs("div", { children: [_jsx(PanelHeader, { icon: "txs", title: "Recent Transactions" }), _jsxs("div", { style: {
                    overflowY: 'auto',
                    maxHeight: 280,
                    scrollbarWidth: 'thin',
                    scrollbarColor: 'rgba(170,112,136,0.3) transparent',
                }, children: [recentTxs.length === 0 && (_jsxs("div", { style: {
                            padding: '24px 14px',
                            fontFamily: 'var(--rd-font-mono)',
                            fontSize: '13px',
                            color: '#6a5a62',
                            textAlign: 'center',
                            lineHeight: 1.6,
                        }, children: ["No transactions yet", _jsx("br", {}), _jsx("span", { style: { fontSize: '11px', color: '#4a4048' }, children: "Chain is producing empty blocks" })] })), _jsx(AnimatePresence, { initial: false, children: recentTxs.map((tx) => {
                            const dot = getDotColor(tx.value);
                            const method = getMethodName(tx.input);
                            return (_jsxs(motion.div, { initial: { opacity: 0, x: 20 }, animate: { opacity: 1, x: 0 }, exit: { opacity: 0, x: 20 }, transition: { duration: 0.3, ease: [0.16, 1, 0.3, 1] }, style: {
                                    display: 'flex',
                                    flexDirection: 'column',
                                    gap: 4,
                                    padding: '8px 14px',
                                    borderBottom: '1px solid rgba(255,255,255,0.04)',
                                    fontFamily: 'var(--rd-font-mono)',
                                    fontSize: '13px',
                                    cursor: 'pointer',
                                    transition: 'background 80ms ease-out',
                                }, onMouseEnter: (e) => {
                                    e.currentTarget.style.background = 'rgba(255,255,255,0.03)';
                                }, onMouseLeave: (e) => {
                                    e.currentTarget.style.background = 'transparent';
                                }, children: [_jsxs("div", { style: {
                                            display: 'flex',
                                            alignItems: 'center',
                                            gap: 8,
                                        }, children: [_jsx("span", { style: {
                                                    width: 6,
                                                    height: 6,
                                                    borderRadius: '50%',
                                                    flexShrink: 0,
                                                    background: dot.color,
                                                    boxShadow: dot.glow ? `0 0 6px ${dot.color}` : undefined,
                                                } }), _jsx("span", { style: { color: '#7a7a98' }, children: truncateAddress(tx.from) }), _jsxs("div", { style: { display: 'flex', alignItems: 'center', gap: 4, flexShrink: 0 }, children: [method && (_jsx("span", { style: {
                                                            fontSize: '9px',
                                                            color: 'var(--rd-text-dim)',
                                                            background: 'rgba(255,255,255,0.04)',
                                                            padding: '1px 4px',
                                                            letterSpacing: '0.04em',
                                                        }, children: method })), _jsxs("svg", { width: "16", height: "10", viewBox: "0 0 16 10", style: { flexShrink: 0 }, children: [_jsx("line", { x1: "0", y1: "5", x2: "12", y2: "5", stroke: "var(--rd-text-ghost)", strokeWidth: "1" }), _jsx("polyline", { points: "10,2 13,5 10,8", fill: "none", stroke: "var(--rd-text-ghost)", strokeWidth: "1" })] })] }), _jsx("span", { style: { color: tx.to ? '#d8c8a0' : '#dca5bd' }, children: tx.to ? truncateAddress(tx.to) : 'CONTRACT CREATE' })] }), tx.value > 0n && (_jsxs("span", { style: { color: '#8a7a82', fontSize: '12px', paddingLeft: 14 }, children: [formatEth(tx.value), " ETH"] }))] }, tx.hash));
                        }) })] })] }));
}
