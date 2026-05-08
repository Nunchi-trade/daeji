import { useEffect, useRef } from 'react';
import { useUIStore } from '@/data/store';
const IDLE_TIMEOUT = 30_000;
const INTERACTION_EVENTS = [
    'mousemove',
    'mousedown',
    'keydown',
    'touchstart',
    'scroll',
    'wheel',
];
export function useAmbientMode() {
    const setAmbient = useUIStore((s) => s.setAmbient);
    const timerRef = useRef(undefined);
    useEffect(() => {
        const resetTimer = () => {
            const state = useUIStore.getState();
            if (state.ambientMode) {
                setAmbient(false);
            }
            clearTimeout(timerRef.current);
            timerRef.current = setTimeout(() => {
                const current = useUIStore.getState();
                if (!current.selectedEntity &&
                    !current.searchOpen &&
                    !current.paused) {
                    setAmbient(true);
                }
            }, IDLE_TIMEOUT);
        };
        for (const event of INTERACTION_EVENTS) {
            window.addEventListener(event, resetTimer, { passive: true });
        }
        resetTimer();
        return () => {
            for (const event of INTERACTION_EVENTS) {
                window.removeEventListener(event, resetTimer);
            }
            clearTimeout(timerRef.current);
        };
    }, [setAmbient]);
}
