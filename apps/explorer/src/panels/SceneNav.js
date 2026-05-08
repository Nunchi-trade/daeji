import { jsx as _jsx, jsxs as _jsxs } from "react/jsx-runtime";
import { useCallback, useEffect } from 'react';
import { useUIStore } from '@/data/store';
const SCENE_TABS = [
    { id: 'terrain', label: 'TERRAIN', key: '1' },
    { id: 'constellation', label: 'CONSTELLATION', key: '2' },
    { id: 'waterfall', label: 'WATERFALL', key: '3' },
    { id: 'consensus', label: 'CONSENSUS', key: '4' },
];
// ---------------------------------------------------------------------------
// Component
// ---------------------------------------------------------------------------
export function SceneNav() {
    const activeScene = useUIStore((s) => s.activeScene);
    const setScene = useUIStore((s) => s.setScene);
    const ambientMode = useUIStore((s) => s.ambientMode);
    // Keyboard: 1-4 switch scenes
    useEffect(() => {
        const handler = (e) => {
            // Don't capture if user is typing in an input
            if (e.target instanceof HTMLInputElement ||
                e.target instanceof HTMLTextAreaElement) {
                return;
            }
            const tab = SCENE_TABS.find((t) => t.key === e.key);
            if (tab) {
                e.preventDefault();
                setScene(tab.id);
            }
        };
        window.addEventListener('keydown', handler);
        return () => window.removeEventListener('keydown', handler);
    }, [setScene]);
    const handleClick = useCallback((id) => {
        setScene(id);
    }, [setScene]);
    return (_jsx("nav", { role: "tablist", "aria-label": "Scene navigation", style: {
            position: 'fixed',
            top: 'calc(48px + var(--rd-space-md))',
            left: 'var(--rd-space-lg)',
            display: 'flex',
            gap: 1,
            zIndex: 'var(--rd-z-panels)',
            opacity: ambientMode ? 0.2 : 1,
            transition: 'opacity 2s ease-out',
            pointerEvents: 'auto',
        }, children: SCENE_TABS.map((tab) => {
            const isActive = activeScene === tab.id;
            return (_jsxs("button", { role: "tab", "aria-selected": isActive, onClick: () => handleClick(tab.id), title: `${tab.label} (${tab.key})`, style: {
                    background: isActive ? 'var(--rd-glass-bg)' : 'transparent',
                    border: '1px solid var(--rd-border)',
                    borderBottom: isActive
                        ? '2px solid var(--rd-rose)'
                        : '1px solid var(--rd-border)',
                    color: isActive ? 'var(--rd-text)' : 'var(--rd-text-dim)',
                    fontFamily: 'var(--rd-font-mono)',
                    fontSize: 'var(--rd-text-xs)',
                    textTransform: 'uppercase',
                    letterSpacing: 'var(--rd-tracking-label)',
                    padding: 'var(--rd-space-xs) var(--rd-space-md)',
                    cursor: 'pointer',
                    backdropFilter: isActive
                        ? 'blur(var(--rd-glass-blur))'
                        : 'none',
                    WebkitBackdropFilter: isActive
                        ? 'blur(var(--rd-glass-blur))'
                        : 'none',
                    transition: `
                color var(--rd-duration-fast) var(--rd-ease-out),
                background var(--rd-duration-fast) var(--rd-ease-out),
                border-color var(--rd-duration-fast) var(--rd-ease-out)
              `,
                }, children: [tab.label, _jsx("span", { style: {
                            marginLeft: 'var(--rd-space-xs)',
                            color: 'var(--rd-text-ghost)',
                            fontSize: 'var(--rd-text-xs)',
                        }, children: tab.key })] }, tab.id));
        }) }));
}
