import { jsx as _jsx } from "react/jsx-runtime";
import { useEffect, useRef } from 'react';
import { bus } from '@/data/bus';
import { GasWaveformRenderer } from './gasWaveformRenderer';
export function GasWaveform() {
    const canvasRef = useRef(null);
    const rendererRef = useRef(null);
    useEffect(() => {
        const canvas = canvasRef.current;
        if (!canvas)
            return;
        const renderer = new GasWaveformRenderer(canvas);
        rendererRef.current = renderer;
        // Resize
        const observer = new ResizeObserver((entries) => {
            for (const entry of entries) {
                renderer.resize(entry.contentRect.width);
            }
        });
        observer.observe(canvas.parentElement);
        // Subscribe to data
        const onBlock = (block) => renderer.pushBlock(block);
        const onFee = (data) => renderer.updateFeeHistory(data);
        bus.on('block:new', onBlock);
        bus.on('fee:update', onFee);
        return () => {
            bus.off('block:new', onBlock);
            bus.off('fee:update', onFee);
            observer.disconnect();
        };
    }, []);
    return (_jsx("canvas", { ref: canvasRef, style: {
            display: 'block',
            width: '100%',
            height: '120px',
            borderTop: '1px solid rgba(255, 255, 255, 0.04)',
        } }));
}
