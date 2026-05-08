import { jsx as _jsx, jsxs as _jsxs } from "react/jsx-runtime";
import { Routes, Route, Navigate } from 'react-router';
import { Suspense, lazy } from 'react';
import { Explorer } from './Explorer';
// Lazy-loaded detail views
const BlockDetail = lazy(() => import('./panels/DetailView/BlockDetail').then((m) => ({
    default: m.BlockDetail,
})));
const TxDetail = lazy(() => import('./panels/DetailView/TxDetail').then((m) => ({
    default: m.TxDetail,
})));
const AddressDetail = lazy(() => import('./panels/DetailView/AddressDetail').then((m) => ({
    default: m.AddressDetail,
})));
function DetailLoading() {
    return (_jsx("div", { style: {
            padding: 'var(--rd-space-lg)',
            fontFamily: 'var(--rd-font-mono)',
            fontSize: 'var(--rd-text-xs)',
            color: 'var(--rd-text-dim)',
            letterSpacing: 'var(--rd-tracking-label)',
            textTransform: 'uppercase',
        }, children: "Loading..." }));
}
export function AppRoutes() {
    return (_jsx(Routes, { children: _jsxs(Route, { path: "/", element: _jsx(Explorer, {}), children: [_jsx(Route, { index: true, element: _jsx(Navigate, { to: "/terrain", replace: true }) }), _jsx(Route, { path: "terrain", element: null }), _jsx(Route, { path: "constellation", element: null }), _jsx(Route, { path: "waterfall", element: null }), _jsx(Route, { path: "consensus", element: null }), _jsx(Route, { path: "block/:numberOrHash", element: _jsx(Suspense, { fallback: _jsx(DetailLoading, {}), children: _jsx(BlockDetail, {}) }) }), _jsx(Route, { path: "tx/:hash", element: _jsx(Suspense, { fallback: _jsx(DetailLoading, {}), children: _jsx(TxDetail, {}) }) }), _jsx(Route, { path: "address/:address", element: _jsx(Suspense, { fallback: _jsx(DetailLoading, {}), children: _jsx(AddressDetail, {}) }) })] }) }));
}
