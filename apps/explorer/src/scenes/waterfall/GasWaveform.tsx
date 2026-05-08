import { useEffect, useRef } from 'react';
import { bus } from '@/data/bus';
import type { ChainBlock, FeeDataPoint } from '@/data/types';
import { GasWaveformRenderer } from './gasWaveformRenderer';

export function GasWaveform() {
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const rendererRef = useRef<GasWaveformRenderer | null>(null);

  useEffect(() => {
    const canvas = canvasRef.current;
    if (!canvas) return;

    const renderer = new GasWaveformRenderer(canvas);
    rendererRef.current = renderer;

    // Resize
    const observer = new ResizeObserver((entries) => {
      for (const entry of entries) {
        renderer.resize(entry.contentRect.width);
      }
    });
    observer.observe(canvas.parentElement!);

    // Subscribe to data
    const onBlock = (block: ChainBlock) => renderer.pushBlock(block);
    const onFee = (data: FeeDataPoint[]) => renderer.updateFeeHistory(data);
    bus.on('block:new', onBlock);
    bus.on('fee:update', onFee);

    return () => {
      bus.off('block:new', onBlock);
      bus.off('fee:update', onFee);
      observer.disconnect();
    };
  }, []);

  return (
    <canvas
      ref={canvasRef}
      style={{
        display: 'block',
        width: '100%',
        height: '120px',
        borderTop: '1px solid rgba(255, 255, 255, 0.04)',
      }}
    />
  );
}
