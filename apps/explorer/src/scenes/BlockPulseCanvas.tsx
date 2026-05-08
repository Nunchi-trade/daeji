import { useRef, useEffect, useCallback } from 'react';
import { bus } from '@/data/bus';
import type { ChainBlock } from '@/data/types';

/**
 * Canvas2D background visualization.
 * Blocks appear as columns with hash-derived patterns.
 * Ambient particles drift across the scene.
 */

interface BlockColumn {
  x: number;
  hash: string;
  gasRatio: number;
  txCount: number;
  hue: number;
  alpha: number;
  birthTime: number;
}

const COL_WIDTH = 4;
const SCROLL_SPEED = 20;
const MAX_COLS = 300;

function hashByte(hash: string, i: number): number {
  const hex = hash.slice(2 + i * 2, 4 + i * 2);
  return parseInt(hex || '80', 16);
}

function hashToHue(hash: string): number {
  let h = 0;
  for (let i = 2; i < 10; i++) {
    h = (h * 31 + (hash.charCodeAt(i) || 0)) | 0;
  }
  return ((h % 60) + 310) % 360;
}

export default function BlockPulseCanvas() {
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const colsRef = useRef<BlockColumn[]>([]);
  const rafRef = useRef<number>(0);
  const lastTimeRef = useRef<number>(0);
  const dprRef = useRef(typeof window !== 'undefined' ? window.devicePixelRatio : 1);

  const addBlock = useCallback((block: ChainBlock) => {
    const gasRatio =
      block.gasLimit > 0n
        ? Number(block.gasUsed) / Number(block.gasLimit)
        : 0;

    colsRef.current.unshift({
      x: 0,
      hash: block.hash,
      gasRatio,
      txCount: block.transactions.length,
      hue: hashToHue(block.hash),
      alpha: 1,
      birthTime: performance.now(),
    });

    if (colsRef.current.length > MAX_COLS) {
      colsRef.current.pop();
    }
  }, []);

  useEffect(() => {
    const canvas = canvasRef.current;
    if (!canvas) return;
    const ctx = canvas.getContext('2d');
    if (!ctx) return;

    bus.on('block:new', addBlock);

    function resize() {
      if (!canvas) return;
      const dpr = window.devicePixelRatio;
      dprRef.current = dpr;
      canvas.width = canvas.offsetWidth * dpr;
      canvas.height = canvas.offsetHeight * dpr;
    }

    resize();
    window.addEventListener('resize', resize);

    function render(time: number) {
      if (!canvas || !ctx) return;
      const dt = lastTimeRef.current ? (time - lastTimeRef.current) / 1000 : 0;
      lastTimeRef.current = time;

      const dpr = dprRef.current;
      const w = canvas.width;
      const h = canvas.height;

      // Clear with void color
      ctx.fillStyle = '#060608';
      ctx.fillRect(0, 0, w, h);

      // === Ambient grid ===
      ctx.globalAlpha = 0.03;
      ctx.strokeStyle = '#aa7088';
      ctx.lineWidth = 1;
      const gridSize = 40 * dpr;
      for (let gx = 0; gx < w; gx += gridSize) {
        ctx.beginPath();
        ctx.moveTo(gx, 0);
        ctx.lineTo(gx, h);
        ctx.stroke();
      }
      for (let gy = 0; gy < h; gy += gridSize) {
        ctx.beginPath();
        ctx.moveTo(0, gy);
        ctx.lineTo(w, gy);
        ctx.stroke();
      }

      // === Block columns ===
      const cols = colsRef.current;
      const colW = COL_WIDTH * dpr;
      const baseY = h * 0.9;

      for (const col of cols) {
        col.x += SCROLL_SPEED * dpr * dt;
        const screenX = w - col.x;
        if (screenX < -colW) {
          col.alpha = 0;
          continue;
        }

        col.alpha = Math.min(1, (screenX + colW) / (w * 0.2));

        // Draw hash-derived column pattern (each byte = one row)
        const rows = 32;
        const rowH = (h * 0.5) / rows;
        const colStartY = baseY - rows * rowH;

        for (let r = 0; r < rows; r++) {
          const byteVal = hashByte(col.hash, r % 16);
          const intensity = byteVal / 255;
          const sat = 30 + intensity * 40;
          const light = 8 + intensity * 25;

          ctx.globalAlpha = col.alpha * (0.3 + intensity * 0.7);
          ctx.fillStyle = `hsl(${col.hue + r * 2}, ${sat}%, ${light}%)`;
          ctx.fillRect(
            screenX,
            colStartY + r * rowH,
            colW,
            rowH - 0.5 * dpr,
          );
        }

        // Bright cap line
        ctx.globalAlpha = col.alpha * 0.9;
        ctx.fillStyle = `hsl(${col.hue}, 50%, 65%)`;
        ctx.fillRect(screenX, colStartY, colW, 2 * dpr);

        // Tx indicator dots
        if (col.txCount > 0) {
          const dots = Math.min(col.txCount, 8);
          ctx.globalAlpha = col.alpha * 0.8;
          ctx.fillStyle = '#d8c8a0';
          for (let d = 0; d < dots; d++) {
            const dy = colStartY + (rows * rowH * (d + 1)) / (dots + 1);
            ctx.beginPath();
            ctx.arc(screenX + colW / 2, dy, 1.5 * dpr, 0, Math.PI * 2);
            ctx.fill();
          }
        }

        // Birth flash
        const age = (time - col.birthTime) / 1000;
        if (age < 0.5) {
          const flash = 1 - age / 0.5;
          ctx.globalAlpha = flash * 0.4;
          ctx.fillStyle = '#dca5bd';
          ctx.fillRect(
            screenX - 2 * dpr,
            colStartY - 4 * dpr,
            colW + 4 * dpr,
            rows * rowH + 8 * dpr,
          );
        }
      }

      // === Baseline ===
      ctx.globalAlpha = 0.08;
      ctx.strokeStyle = '#aa7088';
      ctx.lineWidth = 1 * dpr;
      ctx.beginPath();
      ctx.moveTo(0, baseY);
      ctx.lineTo(w, baseY);
      ctx.stroke();

      // === Floating particles ===
      ctx.globalAlpha = 1;
      const pCount = 50;
      for (let i = 0; i < pCount; i++) {
        const px = ((Math.sin(time * 0.00015 + i * 2.39) + 1) / 2) * w;
        const py = ((Math.cos(time * 0.00012 + i * 1.73) + 1) / 2) * h;
        const pa = 0.04 + Math.sin(time * 0.0008 + i * 0.7) * 0.03;
        const size = (1 + Math.sin(time * 0.001 + i) * 0.5) * dpr;
        ctx.globalAlpha = Math.max(0, pa);
        ctx.fillStyle = i % 3 === 0 ? '#aa7088' : i % 3 === 1 ? '#7a7a98' : '#d8c8a0';
        ctx.beginPath();
        ctx.arc(px, py, size, 0, Math.PI * 2);
        ctx.fill();
      }

      // === Horizontal scan line ===
      const scanY = ((time * 0.03) % h);
      ctx.globalAlpha = 0.04;
      ctx.fillStyle = '#aa7088';
      ctx.fillRect(0, scanY, w, 2 * dpr);

      ctx.globalAlpha = 1;

      // Cleanup dead columns
      colsRef.current = cols.filter((c) => c.alpha > 0);

      rafRef.current = requestAnimationFrame(render);
    }

    rafRef.current = requestAnimationFrame(render);

    return () => {
      bus.off('block:new', addBlock);
      window.removeEventListener('resize', resize);
      cancelAnimationFrame(rafRef.current);
    };
  }, [addBlock]);

  return (
    <canvas
      ref={canvasRef}
      style={{
        position: 'absolute',
        inset: 0,
        width: '100%',
        height: '100%',
      }}
    />
  );
}
