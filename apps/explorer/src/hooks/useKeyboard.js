import { useEffect, useCallback } from 'react';
import { useNavigate, useLocation } from 'react-router';
import { useUIStore } from '@/data/store';
const SCENE_KEYS = {
    '1': 'terrain',
    '2': 'constellation',
    '3': 'waterfall',
    '4': 'consensus',
};
function isInputFocused(event) {
    const target = event.target;
    if (!target)
        return false;
    const tagName = target.tagName.toLowerCase();
    return (tagName === 'input' ||
        tagName === 'textarea' ||
        tagName === 'select' ||
        target.isContentEditable);
}
function parseBigIntSafe(value) {
    try {
        return BigInt(value);
    }
    catch {
        return null;
    }
}
function toggleFullscreen() {
    if (document.fullscreenElement) {
        document.exitFullscreen().catch(() => { });
    }
    else {
        document.documentElement.requestFullscreen().catch(() => { });
    }
}
export function useKeyboard() {
    const navigate = useNavigate();
    const location = useLocation();
    const handleKeyDown = useCallback((event) => {
        const { key, metaKey, ctrlKey } = event;
        const state = useUIStore.getState();
        // Cmd/Ctrl+K: always open search, even when input focused
        if ((metaKey || ctrlKey) && key === 'k') {
            event.preventDefault();
            state.toggleSearch();
            return;
        }
        // Skip all other shortcuts when input is focused
        if (isInputFocused(event))
            return;
        // Escape: contextual close (priority order)
        if (key === 'Escape') {
            event.preventDefault();
            if (state.searchOpen) {
                state.setSearchOpen(false);
                return;
            }
            if (state.selectedEntity) {
                navigate(-1);
                return;
            }
            if (state.ambientMode) {
                state.setAmbient(false);
                return;
            }
            return;
        }
        // Scene switching: 1-4
        if (key in SCENE_KEYS) {
            event.preventDefault();
            const scene = SCENE_KEYS[key];
            state.setScene(scene);
            navigate(`/${scene}`, { replace: false });
            return;
        }
        // Search: /
        if (key === '/') {
            event.preventDefault();
            state.toggleSearch();
            return;
        }
        // Block navigation: arrow keys (only when BlockDetail open)
        if (key === 'ArrowLeft' || key === 'ArrowRight') {
            const path = location.pathname;
            if (!path.startsWith('/block/'))
                return;
            event.preventDefault();
            const current = path.slice('/block/'.length);
            const num = parseBigIntSafe(current);
            if (num === null)
                return;
            const next = key === 'ArrowLeft' ? num - 1n : num + 1n;
            if (next >= 0n) {
                navigate(`/block/${next}`, { replace: true });
            }
            return;
        }
        // Space: toggle pause
        if (key === ' ') {
            event.preventDefault();
            state.togglePause();
            return;
        }
        // F: toggle fullscreen
        if (key === 'f' || key === 'F') {
            event.preventDefault();
            toggleFullscreen();
            return;
        }
    }, [navigate, location.pathname]);
    useEffect(() => {
        window.addEventListener('keydown', handleKeyDown);
        return () => window.removeEventListener('keydown', handleKeyDown);
    }, [handleKeyDown]);
}
