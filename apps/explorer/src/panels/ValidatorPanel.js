import { jsx as _jsx, jsxs as _jsxs, Fragment as _Fragment } from "react/jsx-runtime";
import { useEffect, useState, useRef } from 'react';
import { client } from '@/data/rpc';
import { PanelHeader } from '@/panels/PanelHeader';
import { AnimatedNumber } from '@/lib/AnimatedNumber';
function formatUptime(secs) {
    if (!secs)
        return '--';
    const h = Math.floor(secs / 3600);
    const m = Math.floor((secs % 3600) / 60);
    if (h > 0)
        return `${h}h ${m}m`;
    return `${m}m`;
}
function Metric({ label, value, color, large, }) {
    return (_jsxs("div", { style: { padding: '10px 14px' }, children: [_jsx("div", { style: {
                    fontFamily: 'var(--rd-font-mono)',
                    fontSize: '11px',
                    textTransform: 'uppercase',
                    letterSpacing: '0.08em',
                    color: '#8a7a82',
                    marginBottom: 4,
                }, children: label }), _jsx("div", { style: {
                    fontFamily: 'var(--rd-font-mono)',
                    fontSize: large ? '18px' : '14px',
                    fontWeight: large ? 600 : 400,
                    color: color ?? '#d8c8c4',
                    fontVariantNumeric: 'tabular-nums',
                }, children: value })] }));
}
function ValidatorRing({ validatorCount, validatorIndex, leaderIndex, finalizationRate, }) {
    const cx = 100;
    const cy = 90;
    const r = 70;
    // Arc from 150° to 30° (top half, left to right)
    const startDeg = 150;
    const endDeg = 30;
    // Going clockwise from 150 to 30 means going through 0°: 150 -> 90 -> 30
    // We distribute across 180° of arc (from 150 to 30 going counterclockwise through top)
    const arcSpan = 180; // degrees from 150 down to 30 going counterclockwise (through top)
    const toRad = (deg) => (deg * Math.PI) / 180;
    const getPos = (index) => {
        const t = validatorCount > 1 ? index / (validatorCount - 1) : 0.5;
        // Interpolate from startDeg to endDeg going counterclockwise (decreasing degrees)
        const deg = startDeg - t * arcSpan;
        return {
            x: cx + r * Math.cos(toRad(deg)),
            y: cy - r * Math.sin(toRad(deg)),
        };
    };
    // Progress arc path: same semicircle
    const arcStart = { x: cx + r * Math.cos(toRad(startDeg)), y: cy - r * Math.sin(toRad(startDeg)) };
    const arcEnd = { x: cx + r * Math.cos(toRad(endDeg)), y: cy - r * Math.sin(toRad(endDeg)) };
    const bgArcPath = `M ${arcStart.x} ${arcStart.y} A ${r} ${r} 0 0 1 ${arcEnd.x} ${arcEnd.y}`;
    // Circumference of the 180° arc
    const arcLength = Math.PI * r; // half circumference
    const filledLength = arcLength * Math.min(1, Math.max(0, finalizationRate));
    return (_jsxs("svg", { viewBox: "0 0 200 110", width: "100%", style: { display: 'block', margin: '0 auto' }, children: [_jsx("path", { d: bgArcPath, fill: "none", stroke: "var(--rd-text-ghost, #3a303a)", strokeWidth: 3, strokeLinecap: "round" }), _jsx("path", { d: bgArcPath, fill: "none", stroke: "var(--rd-success, #7a8a78)", strokeWidth: 3, strokeLinecap: "round", strokeDasharray: `${filledLength} ${arcLength}`, strokeDashoffset: 0 }), Array.from({ length: validatorCount }, (_, i) => {
                const { x, y } = getPos(i);
                const isLocal = i === validatorIndex;
                const isLeader = i === leaderIndex;
                const color = isLeader ? '#dca5bd' : isLocal ? '#6abf69' : '#7a7a98';
                return (_jsxs("g", { children: [isLeader && (_jsx("circle", { cx: x, cy: y, r: 14, fill: color, fillOpacity: 0.3, children: _jsx("animate", { attributeName: "opacity", values: "0.4;1;0.4", dur: "2s", repeatCount: "indefinite" }) })), _jsx("circle", { cx: x, cy: y, r: 8, fill: color }), _jsxs("text", { x: x, y: y + 20, textAnchor: "middle", fontFamily: "var(--rd-font-mono, monospace)", fontSize: 7, fill: color, opacity: 0.8, children: ["V", i] })] }, i));
            })] }));
}
async function fetchNodeStatus() {
    try {
        const result = await client.request({
            method: 'kora_nodeStatus',
            params: [],
        });
        const raw = result;
        return {
            chainId: Number(raw.chainId),
            validatorIndex: Number(raw.validatorIndex),
            validatorCount: Number(raw.validatorCount ?? 3),
            uptimeSecs: Number(raw.uptimeSecs ?? 0),
            currentView: Number(raw.currentView),
            finalizedCount: Number(raw.finalizedCount),
            proposedCount: Number(raw.proposedCount),
            nullifiedCount: Number(raw.nullifiedCount),
            peerCount: Number(raw.peerCount),
            isLeader: Boolean(raw.isLeader),
        };
    }
    catch {
        return null;
    }
}
export function ValidatorPanel() {
    const [status, setStatus] = useState(null);
    const retriesRef = useRef(0);
    useEffect(() => {
        let alive = true;
        async function poll() {
            // Retry up to 5 times quickly to handle Railway load balancing
            // (some validators don't have kora_nodeStatus)
            const result = await fetchNodeStatus();
            if (!alive)
                return;
            if (result) {
                setStatus(result);
                retriesRef.current = 0;
            }
            else if (retriesRef.current < 5) {
                retriesRef.current++;
                // Quick retry in 500ms
                setTimeout(poll, 500);
                return;
            }
        }
        poll();
        const id = setInterval(poll, 4_000);
        return () => {
            alive = false;
            clearInterval(id);
        };
    }, []);
    const liveBadge = (_jsxs("span", { style: {
            fontSize: '10px',
            color: '#6abf69',
            display: 'flex',
            alignItems: 'center',
            gap: 4,
        }, children: [_jsx("span", { style: {
                    width: 6,
                    height: 6,
                    borderRadius: '50%',
                    background: '#6abf69',
                    boxShadow: '0 0 6px #6abf69',
                    display: 'inline-block',
                } }), "LIVE"] }));
    if (!status) {
        return (_jsxs("div", { children: [_jsx(PanelHeader, { icon: "consensus", title: "Consensus" }), _jsx("div", { style: {
                        padding: '20px 14px',
                        fontFamily: 'var(--rd-font-mono)',
                        fontSize: '12px',
                        color: '#6a5a62',
                        textAlign: 'center',
                    }, children: "Connecting to validators..." })] }));
    }
    const finalizationRate = status.currentView > 0 ? status.finalizedCount / status.currentView : 0;
    const leaderIndex = status.currentView % (status.validatorCount || 1);
    const metricValueStyle = {
        fontFamily: 'var(--rd-font-mono)',
        fontSize: '14px',
        fontWeight: 400,
        color: '#d8c8c4',
        fontVariantNumeric: 'tabular-nums',
    };
    const metricValueLargeStyle = {
        fontFamily: 'var(--rd-font-mono)',
        fontSize: '18px',
        fontWeight: 600,
        color: '#dca5bd',
        fontVariantNumeric: 'tabular-nums',
    };
    return (_jsxs("div", { children: [_jsx(PanelHeader, { icon: "consensus", title: "Consensus", badge: liveBadge }), _jsx("div", { style: { borderBottom: '1px solid rgba(255,255,255,0.06)', padding: '8px 0 4px' }, children: _jsx(ValidatorRing, { validatorCount: status.validatorCount || 3, validatorIndex: status.validatorIndex, leaderIndex: leaderIndex, finalizationRate: finalizationRate }) }), _jsxs("div", { style: {
                    display: 'grid',
                    gridTemplateColumns: '1fr 1fr',
                    gap: '1px',
                    background: 'rgba(255,255,255,0.04)',
                }, children: [_jsx("div", { style: { background: 'rgba(12, 12, 16, 0.9)' }, children: _jsx(Metric, { label: "Current View", value: _jsx(AnimatedNumber, { value: status.currentView, format: (n) => Math.round(n).toLocaleString(), style: metricValueLargeStyle }), color: "#dca5bd", large: true }) }), _jsx("div", { style: { background: 'rgba(12, 12, 16, 0.9)' }, children: _jsx(Metric, { label: "Peers", value: _jsxs(_Fragment, { children: [_jsx(AnimatedNumber, { value: status.peerCount, format: (n) => Math.round(n).toLocaleString(), style: metricValueLargeStyle }), ' / ', status.validatorCount] }), large: true }) }), _jsx("div", { style: { background: 'rgba(12, 12, 16, 0.9)' }, children: _jsx(Metric, { label: "Finalized", value: _jsx(AnimatedNumber, { value: status.finalizedCount, format: (n) => Math.round(n).toLocaleString(), style: { ...metricValueStyle, color: '#6abf69' } }), color: "#6abf69" }) }), _jsx("div", { style: { background: 'rgba(12, 12, 16, 0.9)' }, children: _jsx(Metric, { label: "Finalization Rate", value: `${status.currentView > 0 ? ((status.finalizedCount / status.currentView) * 100).toFixed(1) : '0'}%`, color: "#6abf69" }) }), _jsx("div", { style: { background: 'rgba(12, 12, 16, 0.9)' }, children: _jsx(Metric, { label: "Proposed", value: _jsx(AnimatedNumber, { value: status.proposedCount, format: (n) => Math.round(n).toLocaleString(), style: metricValueStyle }) }) }), _jsx("div", { style: { background: 'rgba(12, 12, 16, 0.9)' }, children: _jsx(Metric, { label: "Nullified", value: _jsx(AnimatedNumber, { value: status.nullifiedCount, format: (n) => Math.round(n).toLocaleString(), style: {
                                    ...metricValueStyle,
                                    color: status.nullifiedCount > 0 ? '#e8a84c' : undefined,
                                } }), color: status.nullifiedCount > 0 ? '#e8a84c' : undefined }) }), _jsx("div", { style: { background: 'rgba(12, 12, 16, 0.9)' }, children: _jsx(Metric, { label: "Uptime", value: formatUptime(status.uptimeSecs) }) }), _jsx("div", { style: { background: 'rgba(12, 12, 16, 0.9)' }, children: _jsx(Metric, { label: "Chain ID", value: status.chainId }) })] })] }));
}
