import { jsx as _jsx, jsxs as _jsxs } from "react/jsx-runtime";
import { Outlet, useLocation } from 'react-router';
import { useEffect, useMemo, lazy, Suspense } from 'react';
import { AnimatePresence, motion } from 'framer-motion';
import { useKeyboard } from '@/hooks/useKeyboard';
import { StatusBar } from '@/panels/StatusBar';
import { BlockFeed } from '@/panels/BlockFeed';
import { ChainStats } from '@/panels/ChainStats';
import { TxFeed } from '@/panels/TxFeed';
import { ValidatorPanel } from '@/panels/ValidatorPanel';
import { ConsensusTimeline } from '@/panels/ConsensusTimeline';
import { BlockFullness } from '@/panels/BlockFullness';
import { SearchOverlay } from '@/panels/SearchOverlay';
import { ConnectionManager } from '@/data/connection';
import { BlockPoller } from '@/data/poller';
import { bus } from '@/data/bus';
const BlockPulseCanvas = lazy(() => import('@/scenes/BlockPulseCanvas'));
const connection = new ConnectionManager();
const poller = new BlockPoller();
// Glass panel wrapper
function GlassPanel({ children, style, }) {
    return (_jsx("div", { style: {
            background: 'rgba(12, 12, 16, 0.85)',
            backdropFilter: 'blur(12px) saturate(1.4)',
            WebkitBackdropFilter: 'blur(12px) saturate(1.4)',
            border: '1px solid var(--rd-border)',
            boxShadow: 'inset 0 1px 0 rgba(255,255,255,0.03)',
            overflow: 'hidden',
            ...style,
        }, children: children }));
}
export function Explorer() {
    const location = useLocation();
    useKeyboard();
    // Start connection + polling on mount
    useEffect(() => {
        let statusInterval = null;
        connection.onConnect = () => {
            poller.start();
            statusInterval = setInterval(async () => {
                try {
                    const { getNodeStatus } = await import('@/data/rpc');
                    const status = await getNodeStatus();
                    bus.emit('node:status', status);
                }
                catch {
                    // kora_nodeStatus may not be deployed
                }
            }, 10_000);
        };
        connection.onDisconnect = () => {
            poller.stop();
            if (statusInterval)
                clearInterval(statusInterval);
            statusInterval = null;
        };
        connection.connect();
        return () => {
            if (statusInterval)
                clearInterval(statusInterval);
            poller.stop();
            connection.disconnect();
        };
    }, []);
    const isDetailRoute = useMemo(() => {
        const path = location.pathname;
        return (path.startsWith('/block/') ||
            path.startsWith('/tx/') ||
            path.startsWith('/address/'));
    }, [location.pathname]);
    return (_jsxs("div", { className: "explorer", style: {
            position: 'fixed',
            inset: 0,
            overflow: 'hidden',
            background: 'var(--rd-void)',
            display: 'flex',
            flexDirection: 'column',
        }, children: [_jsx("div", { className: "grain", "aria-hidden": "true" }), _jsx("div", { className: "roseWash", "aria-hidden": "true" }), _jsx("div", { style: {
                    position: 'fixed',
                    inset: 0,
                    zIndex: 0,
                    opacity: 0.6,
                }, "aria-hidden": "true", children: _jsx(Suspense, { fallback: null, children: _jsx(BlockPulseCanvas, {}) }) }), _jsx(StatusBar, {}), _jsxs("div", { style: {
                    position: 'relative',
                    zIndex: 1,
                    flex: 1,
                    display: 'grid',
                    gridTemplateColumns: '1fr 360px',
                    gap: 'var(--rd-space-md)',
                    padding: 'var(--rd-space-md)',
                    paddingTop: 'calc(48px + var(--rd-space-md))',
                    overflow: 'hidden',
                    minHeight: 0,
                }, children: [_jsx("div", { style: {
                            display: 'flex',
                            flexDirection: 'column',
                            gap: 'var(--rd-space-md)',
                            minHeight: 0,
                            overflow: 'hidden',
                        }, children: _jsx(GlassPanel, { style: { flex: 1, overflow: 'hidden', display: 'flex', flexDirection: 'column' }, children: _jsx(BlockFeed, {}) }) }), _jsxs("div", { style: {
                            display: 'flex',
                            flexDirection: 'column',
                            gap: 'var(--rd-space-md)',
                            minHeight: 0,
                            overflow: 'auto',
                            scrollbarWidth: 'thin',
                            scrollbarColor: 'var(--rd-text-ghost) transparent',
                        }, children: [_jsx(GlassPanel, { children: _jsx(ValidatorPanel, {}) }), _jsx(GlassPanel, { children: _jsx(ConsensusTimeline, {}) }), _jsx(GlassPanel, { children: _jsx(ChainStats, {}) }), _jsx(GlassPanel, { children: _jsx(BlockFullness, {}) }), _jsx(GlassPanel, { children: _jsx(TxFeed, {}) })] })] }), _jsx(AnimatePresence, { mode: "wait", children: isDetailRoute && (_jsx(motion.div, { initial: { opacity: 0, x: '100%' }, animate: { opacity: 1, x: 0 }, exit: { opacity: 0, x: '100%' }, transition: { duration: 0.4, ease: [0.16, 1, 0.3, 1] }, style: {
                        position: 'fixed',
                        top: 0,
                        right: 0,
                        bottom: 0,
                        width: 'clamp(400px, 50vw, 720px)',
                        zIndex: 10,
                        overflowY: 'auto',
                        background: 'rgba(12, 12, 16, 0.92)',
                        backdropFilter: 'blur(16px) saturate(1.4)',
                        WebkitBackdropFilter: 'blur(16px) saturate(1.4)',
                        borderLeft: '1px solid var(--rd-border)',
                    }, role: "dialog", "aria-label": "Entity detail", "aria-modal": "true", children: _jsx(Outlet, {}) }, location.pathname)) }), _jsx(SearchOverlay, {}), _jsx("style", { children: `
        @keyframes fadeSlideIn {
          from { opacity: 0; transform: translateY(-4px); }
          to { opacity: 1; transform: translateY(0); }
        }
      ` })] }));
}
