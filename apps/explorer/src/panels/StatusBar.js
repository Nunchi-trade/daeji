import { jsx as _jsx, jsxs as _jsxs } from "react/jsx-runtime";
import { useChainStore, useUIStore } from '@/data/store';
import { kora } from '@/data/rpc';
import { formatBlockNumber } from '@/lib/format';
import { FeeWaveform } from './FeeWaveform';
import { ActivityPulse } from './ActivityPulse';
import { NetworkHealth } from './NetworkHealth';
import { RpcLatency } from './RpcLatency';
import styles from '@/design/glass.module.css';
// ---------------------------------------------------------------------------
// Connection LED
// ---------------------------------------------------------------------------
function ConnectionLed() {
    const connectionState = useChainStore((s) => s.connectionState);
    const ledColor = connectionState === 'connected'
        ? 'var(--rd-led-connected)'
        : connectionState === 'reconnecting'
            ? 'var(--rd-led-warning)'
            : 'var(--rd-led-error)';
    const label = connectionState === 'connected'
        ? 'CONNECTED'
        : connectionState === 'reconnecting'
            ? 'RECONNECTING'
            : 'DISCONNECTED';
    return (_jsxs("div", { style: {
            display: 'flex',
            alignItems: 'center',
            gap: 'var(--rd-space-xs)',
        }, "aria-live": "polite", children: [_jsx("span", { className: styles.led, style: {
                    background: ledColor,
                    boxShadow: `0 0 6px ${ledColor}`,
                    animation: connectionState === 'reconnecting'
                        ? 'pulse 1.2s ease-in-out infinite'
                        : undefined,
                } }), _jsx("span", { className: styles.label, children: label })] }));
}
// ---------------------------------------------------------------------------
// Chain ID badge
// ---------------------------------------------------------------------------
function ChainIdBadge() {
    return (_jsx("span", { style: {
            fontFamily: 'var(--rd-font-mono)',
            fontSize: 'var(--rd-text-xs)',
            color: 'var(--rd-text-dim)',
            letterSpacing: 'var(--rd-tracking-normal)',
        }, children: kora.id }));
}
// ---------------------------------------------------------------------------
// Block counter with pulse on new block
// ---------------------------------------------------------------------------
function BlockCounter() {
    const latestBlockNum = useChainStore((s) => s.latestBlock);
    const blocks = useChainStore((s) => s.blocks);
    const latestBlock = latestBlockNum > 0n ? blocks.get(latestBlockNum) : undefined;
    let blockTimeLabel = '';
    if (latestBlock && latestBlock._arrivalTime) {
        const elapsed = Date.now() - latestBlock._arrivalTime;
        if (elapsed < 60_000) {
            blockTimeLabel = `${Math.round(elapsed / 1000)}s ago`;
        }
    }
    return (_jsxs("div", { style: {
            display: 'flex',
            alignItems: 'center',
            gap: 'var(--rd-space-sm)',
        }, children: [_jsxs("span", { style: {
                    fontFamily: 'var(--rd-font-mono)',
                    fontSize: 'var(--rd-text-sm)',
                    color: 'var(--rd-rose)',
                    letterSpacing: 'var(--rd-tracking-normal)',
                    fontVariantNumeric: 'tabular-nums',
                }, children: ["#", formatBlockNumber(latestBlockNum)] }), blockTimeLabel && (_jsx("span", { style: {
                    fontFamily: 'var(--rd-font-mono)',
                    fontSize: 'var(--rd-text-xs)',
                    color: 'var(--rd-text-dim)',
                }, children: blockTimeLabel }))] }));
}
// ---------------------------------------------------------------------------
// Gas price display
// ---------------------------------------------------------------------------
function GasPrice() {
    const latestBlockNum = useChainStore((s) => s.latestBlock);
    const blocks = useChainStore((s) => s.blocks);
    const latestBlock = latestBlockNum > 0n ? blocks.get(latestBlockNum) : undefined;
    if (!latestBlock)
        return null;
    const gweiStr = (Number(latestBlock.baseFeePerGas) / 1e9).toFixed(2);
    return (_jsxs("span", { style: {
            fontFamily: 'var(--rd-font-mono)',
            fontSize: 'var(--rd-text-xs)',
            color: 'var(--rd-warning)',
            fontVariantNumeric: 'tabular-nums',
        }, children: [gweiStr, ' ', _jsx("span", { style: { color: 'var(--rd-text-dim)' }, children: "gwei" })] }));
}
// ---------------------------------------------------------------------------
// Peer count (from consensus store node status updates)
// ---------------------------------------------------------------------------
function PeerCount() {
    // Peer count comes through consensus store's updateFromNodeStatus;
    // the validator count serves as a proxy when full peer count isn't exposed.
    const validatorCount = useChainStore(() => 0); // placeholder
    if (!validatorCount)
        return null;
    return (_jsxs("span", { style: {
            fontFamily: 'var(--rd-font-mono)',
            fontSize: 'var(--rd-text-xs)',
            color: 'var(--rd-text-dim)',
            fontVariantNumeric: 'tabular-nums',
        }, children: [validatorCount, ' ', _jsx("span", { style: { letterSpacing: 'var(--rd-tracking-label)' }, children: "PEERS" })] }));
}
// ---------------------------------------------------------------------------
// Main Component
// ---------------------------------------------------------------------------
export function StatusBar() {
    const ambientMode = useUIStore((s) => s.ambientMode);
    return (_jsxs("div", { style: {
            position: 'fixed',
            left: 0,
            right: 0,
            top: 0,
            height: 48,
            background: 'var(--rd-void-surface)',
            borderBottom: '1px solid var(--rd-border)',
            display: 'flex',
            alignItems: 'center',
            justifyContent: 'space-between',
            padding: '0 var(--rd-space-md)',
            gap: 'var(--rd-space-md)',
            zIndex: 'var(--rd-z-panels)',
            opacity: ambientMode ? 0.2 : 1,
            transition: 'opacity 2s ease-out',
            pointerEvents: 'auto',
        }, role: "status", "aria-label": "Explorer status bar", children: [_jsxs("div", { style: {
                    display: 'flex',
                    alignItems: 'center',
                    gap: 'var(--rd-space-sm)',
                }, children: [_jsx(ConnectionLed, {}), _jsx(ChainIdBadge, {})] }), _jsxs("div", { style: {
                    display: 'flex',
                    alignItems: 'center',
                    gap: 'var(--rd-space-md)',
                }, children: [_jsx(BlockCounter, {}), _jsx(ActivityPulse, {}), _jsx(FeeWaveform, { width: 120, height: 24 }), _jsx(GasPrice, {})] }), _jsxs("div", { style: { display: 'flex', alignItems: 'center', gap: 'var(--rd-space-md)' }, children: [_jsx(NetworkHealth, {}), _jsx(RpcLatency, {}), _jsx(PeerCount, {})] })] }));
}
