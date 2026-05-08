import { jsxs as _jsxs, jsx as _jsx } from "react/jsx-runtime";
import { Suspense, lazy, memo } from 'react';
// Lazy-loaded scenes (code-split per scene)
const scenes = {
    terrain: lazy(() => import('./terrain/TerrainScene')),
    constellation: lazy(() => import('./constellation/ConstellationCanvas')),
    waterfall: lazy(() => import('./waterfall/WaterfallScene')),
    consensus: lazy(() => import('./consensus/ConsensusScene')),
};
function SceneLoading({ scene }) {
    return (_jsxs("div", { style: {
            position: 'absolute',
            inset: 0,
            display: 'flex',
            alignItems: 'center',
            justifyContent: 'center',
            fontFamily: 'var(--rd-font-mono)',
            fontSize: 'var(--rd-text-xs)',
            color: 'var(--rd-text-dim)',
            letterSpacing: 'var(--rd-tracking-label)',
            textTransform: 'uppercase',
        }, children: ["Loading ", scene] }));
}
function SceneError({ scene }) {
    return (_jsxs("div", { style: {
            position: 'absolute',
            inset: 0,
            display: 'flex',
            alignItems: 'center',
            justifyContent: 'center',
            fontFamily: 'var(--rd-font-mono)',
            fontSize: 'var(--rd-text-xs)',
            color: 'var(--rd-danger, #ff4444)',
            letterSpacing: 'var(--rd-tracking-label)',
            textTransform: 'uppercase',
        }, children: ["Failed to load ", scene] }));
}
// Minimal error boundary for scene rendering
import { Component } from 'react';
class SceneErrorBoundary extends Component {
    state = { hasError: false };
    static getDerivedStateFromError() {
        return { hasError: true };
    }
    componentDidCatch(error, info) {
        console.error('[SceneManager] Error rendering scene:', error, info);
    }
    render() {
        if (this.state.hasError)
            return this.props.fallback;
        return this.props.children;
    }
}
export const SceneManager = memo(function SceneManager({ activeScene, }) {
    const SceneComponent = scenes[activeScene];
    return (_jsx(SceneErrorBoundary, { fallback: _jsx(SceneError, { scene: activeScene }), children: _jsx(Suspense, { fallback: _jsx(SceneLoading, { scene: activeScene }), children: _jsx(SceneComponent, {}) }) }));
});
