import { jsx as _jsx } from "react/jsx-runtime";
import { useEffect, useRef } from 'react';
import { bus } from '@/data/bus';
const DOT_COUNT = 12;
const DOT_SPACING = 16; // px between dots
const DOT_RADIUS = 2;
const FLASH_DURATION = 800; // ms for the flash decay
/**
 * Block pulse heartbeat indicator.
 * One dot per second, flashes rose-glow when a block arrives.
 */
export function BlockPulse() {
    const canvasRef = useRef(null);
    const lastBlockTimeRef = useRef(0);
    const rafRef = useRef(0);
    useEffect(() => {
        const canvas = canvasRef.current;
        if (!canvas)
            return;
        const ctx = canvas.getContext('2d');
        const dpr = Math.min(window.devicePixelRatio || 1, 2);
        const totalWidth = DOT_COUNT * DOT_SPACING;
        canvas.width = totalWidth * dpr;
        canvas.height = 24 * dpr;
        canvas.style.width = `${totalWidth}px`;
        canvas.style.height = '24px';
        ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
        const onBlock = () => {
            lastBlockTimeRef.current = Date.now();
        };
        bus.on('block:new', onBlock);
        const render = () => {
            const now = Date.now();
            const secondFrac = (now % 1000) / 1000;
            const timeSinceBlock = now - lastBlockTimeRef.current;
            const flashFactor = Math.max(0, 1 - timeSinceBlock / FLASH_DURATION);
            ctx.clearRect(0, 0, totalWidth, 24);
            // Current dot index: cycles based on time
            const activeDot = Math.floor((now / 1000) % DOT_COUNT);
            for (let i = 0; i < DOT_COUNT; i++) {
                const x = DOT_SPACING / 2 + i * DOT_SPACING;
                const y = 12;
                const isActive = i === activeDot;
                const isPast = i < activeDot;
                ctx.beginPath();
                ctx.arc(x, y, DOT_RADIUS, 0, Math.PI * 2);
                if (isActive && flashFactor > 0) {
                    // Flash rose-glow when a block just arrived
                    const alpha = 0.3 + flashFactor * 0.7;
                    ctx.fillStyle = `rgba(220, 165, 189, ${alpha})`; // --rd-rose-glow
                    // Glow effect
                    ctx.shadowColor = '#dca5bd';
                    ctx.shadowBlur = 6 * flashFactor;
                    ctx.fill();
                    ctx.shadowBlur = 0;
                }
                else if (isActive) {
                    // Active dot: pulsing based on second fraction
                    const pulse = 0.4 + Math.sin(secondFrac * Math.PI) * 0.3;
                    ctx.fillStyle = `rgba(170, 112, 136, ${pulse})`; // --rd-rose
                    ctx.fill();
                }
                else if (isPast) {
                    // Past dots: dim
                    ctx.fillStyle = 'rgba(106, 90, 104, 0.3)'; // --rd-text-dim
                    ctx.fill();
                }
                else {
                    // Future dots: very dim
                    ctx.fillStyle = 'rgba(106, 90, 104, 0.12)';
                    ctx.fill();
                }
            }
            // Connecting line between dots
            ctx.beginPath();
            ctx.moveTo(DOT_SPACING / 2, 12);
            ctx.lineTo(DOT_SPACING / 2 + (DOT_COUNT - 1) * DOT_SPACING, 12);
            ctx.strokeStyle = 'rgba(255, 255, 255, 0.04)';
            ctx.lineWidth = 1;
            ctx.stroke();
            rafRef.current = requestAnimationFrame(render);
        };
        rafRef.current = requestAnimationFrame(render);
        return () => {
            cancelAnimationFrame(rafRef.current);
            bus.off('block:new', onBlock);
        };
    }, []);
    return (_jsx("div", { style: { display: 'flex', justifyContent: 'center', padding: '4px 0' }, children: _jsx("canvas", { ref: canvasRef, style: { display: 'block' } }) }));
}
