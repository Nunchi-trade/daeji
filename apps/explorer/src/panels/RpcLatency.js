import { jsx as _jsx, jsxs as _jsxs } from "react/jsx-runtime";
import { useChainStore } from '@/data/store';
function latencyColor(ms) {
    if (ms < 100)
        return '#7a8a78';
    if (ms <= 500)
        return '#c89a68';
    return '#cc5555';
}
function latencyFillWidth(ms) {
    const clamped = Math.min(ms, 1000);
    return (clamped / 1000) * 100;
}
export function RpcLatency() {
    const rpcLatency = useChainStore((s) => s.rpcLatency);
    if (rpcLatency === 0)
        return null;
    const color = latencyColor(rpcLatency);
    const fillPct = latencyFillWidth(rpcLatency);
    return (_jsxs("div", { style: {
            display: 'flex',
            flexDirection: 'row',
            alignItems: 'center',
            gap: 6,
        }, children: [_jsx("div", { style: {
                    width: 24,
                    height: 4,
                    borderRadius: 2,
                    background: '#3a303a',
                    overflow: 'hidden',
                }, children: _jsx("div", { style: {
                        width: `${fillPct}%`,
                        height: '100%',
                        background: color,
                        borderRadius: 2,
                    } }) }), _jsxs("span", { style: {
                    fontFamily: 'var(--rd-font-mono)',
                    fontSize: 10,
                    color,
                    fontVariantNumeric: 'tabular-nums',
                }, children: [Math.round(rpcLatency), "ms"] })] }));
}
