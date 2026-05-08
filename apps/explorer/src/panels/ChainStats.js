import { jsx as _jsx, jsxs as _jsxs } from "react/jsx-runtime";
import { useChainStore } from '@/data/store';
import { AnimatedNumber } from '@/lib/AnimatedNumber';
import { PanelHeader } from '@/panels/PanelHeader';
import { BlockTimeSparkline } from '@/panels/charts/BlockTimeSparkline';
import { FeeTrendChart } from '@/panels/charts/FeeTrendChart';
import { GasUsageChart } from '@/panels/charts/GasUsageChart';
import { TxVolumeChart } from '@/panels/charts/TxVolumeChart';
function Stat({ label, value, color, }) {
    return (_jsxs("div", { style: { padding: '10px 14px' }, children: [_jsx("div", { style: {
                    fontFamily: 'var(--rd-font-mono)',
                    fontSize: '11px',
                    textTransform: 'uppercase',
                    letterSpacing: '0.08em',
                    color: '#8a7a82',
                    marginBottom: 4,
                }, children: label }), _jsx("div", { style: {
                    fontFamily: 'var(--rd-font-mono)',
                    fontSize: '14px',
                    color: color ?? '#d8c8c4',
                    fontVariantNumeric: 'tabular-nums',
                }, children: value })] }));
}
const chartLabelStyle = {
    fontFamily: 'var(--rd-font-mono)',
    fontSize: '10px',
    textTransform: 'uppercase',
    letterSpacing: '0.08em',
    color: '#8a7a82',
    padding: '8px 14px 4px',
};
export function ChainStats() {
    const latestBlock = useChainStore((s) => s.latestBlock);
    const blocks = useChainStore((s) => s.blocks);
    const blockOrder = useChainStore((s) => s.blockOrder);
    let totalTxs = 0;
    let avgGasRatio = 0;
    const recentBlocks = blockOrder.slice(0, 20);
    for (const num of recentBlocks) {
        const b = blocks.get(num);
        if (!b)
            continue;
        totalTxs += b.transactions.length;
        if (b.gasLimit > 0n) {
            avgGasRatio += Number(b.gasUsed) / Number(b.gasLimit);
        }
    }
    const avgGas = recentBlocks.length > 0
        ? ((avgGasRatio / recentBlocks.length) * 100).toFixed(1) + '%'
        : '--';
    const latestBlockData = latestBlock > 0n ? blocks.get(latestBlock) : undefined;
    const baseFee = latestBlockData
        ? (Number(latestBlockData.baseFeePerGas) / 1e9).toFixed(2) + ' gwei'
        : '--';
    return (_jsxs("div", { children: [_jsx(PanelHeader, { icon: "stats", title: "Chain Stats" }), _jsxs("div", { style: {
                    display: 'grid',
                    gridTemplateColumns: '1fr 1fr',
                    gap: '1px',
                    background: 'rgba(255,255,255,0.04)',
                }, children: [_jsxs("div", { style: { background: 'rgba(12, 12, 16, 0.9)', padding: '10px 14px' }, children: [_jsx("div", { style: {
                                    fontFamily: 'var(--rd-font-mono)',
                                    fontSize: '11px',
                                    textTransform: 'uppercase',
                                    letterSpacing: '0.08em',
                                    color: '#8a7a82',
                                    marginBottom: 4,
                                }, children: "Block Height" }), latestBlock > 0n ? (_jsx(AnimatedNumber, { value: Number(latestBlock), format: (n) => '#' + Math.round(n).toLocaleString(), style: { fontFamily: 'var(--rd-font-mono)', fontSize: '14px', color: '#dca5bd' } })) : (_jsx("div", { style: { fontFamily: 'var(--rd-font-mono)', fontSize: '14px', color: '#dca5bd' }, children: "--" }))] }), _jsxs("div", { style: { background: 'rgba(12, 12, 16, 0.9)', padding: '10px 14px' }, children: [_jsx("div", { style: {
                                    fontFamily: 'var(--rd-font-mono)',
                                    fontSize: '11px',
                                    textTransform: 'uppercase',
                                    letterSpacing: '0.08em',
                                    color: '#8a7a82',
                                    marginBottom: 4,
                                }, children: "Block Time" }), _jsx(BlockTimeSparkline, { width: 140, height: 32 })] }), _jsx("div", { style: { background: 'rgba(12, 12, 16, 0.9)' }, children: _jsx(Stat, { label: "Base Fee", value: baseFee, color: "#e8a84c" }) }), _jsx("div", { style: { background: 'rgba(12, 12, 16, 0.9)' }, children: _jsx(Stat, { label: "Avg Gas Usage", value: avgGas }) }), _jsxs("div", { style: { background: 'rgba(12, 12, 16, 0.9)', padding: '10px 14px' }, children: [_jsx("div", { style: {
                                    fontFamily: 'var(--rd-font-mono)',
                                    fontSize: '11px',
                                    textTransform: 'uppercase',
                                    letterSpacing: '0.08em',
                                    color: '#8a7a82',
                                    marginBottom: 4,
                                }, children: "Recent TXs" }), _jsx(AnimatedNumber, { value: totalTxs, style: { fontFamily: 'var(--rd-font-mono)', fontSize: '14px', color: '#d8c8c4' } })] }), _jsxs("div", { style: { background: 'rgba(12, 12, 16, 0.9)', padding: '10px 14px' }, children: [_jsx("div", { style: {
                                    fontFamily: 'var(--rd-font-mono)',
                                    fontSize: '11px',
                                    textTransform: 'uppercase',
                                    letterSpacing: '0.08em',
                                    color: '#8a7a82',
                                    marginBottom: 4,
                                }, children: "Gas Used" }), _jsx(GasUsageChart, { width: 140, height: 32 })] })] }), _jsxs("div", { children: [_jsx("div", { style: chartLabelStyle, children: "Base Fee Trend" }), _jsx(FeeTrendChart, {})] }), _jsxs("div", { children: [_jsx("div", { style: chartLabelStyle, children: "Tx Volume" }), _jsx(TxVolumeChart, {})] })] }));
}
