import { jsx as _jsx } from "react/jsx-runtime";
import { useEffect, useRef, useState } from 'react';
import { bus } from '@/data/bus';
export function ActivityPulse() {
    const [active, setActive] = useState(false);
    const timeoutRef = useRef(null);
    useEffect(() => {
        const handler = () => {
            setActive(true);
            if (timeoutRef.current)
                clearTimeout(timeoutRef.current);
            timeoutRef.current = setTimeout(() => setActive(false), 600);
        };
        bus.on('block:new', handler);
        return () => {
            bus.off('block:new', handler);
            if (timeoutRef.current)
                clearTimeout(timeoutRef.current);
        };
    }, []);
    return (_jsx("svg", { width: 16, height: 16, viewBox: "0 0 16 16", style: {
            opacity: active ? 1 : 0.4,
            transform: active ? 'scale(1.15)' : 'scale(1)',
            transition: 'opacity 600ms ease-out, transform 200ms ease-out',
        }, children: _jsx("path", { d: "M 0 8 L 4 8 L 6 3 L 8 13 L 10 8 L 16 8", stroke: "var(--rd-rose)", strokeWidth: 1.5, strokeLinecap: "round", fill: "none" }) }));
}
