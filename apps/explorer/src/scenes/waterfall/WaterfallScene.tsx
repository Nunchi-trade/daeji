import { useRef, useEffect, useCallback } from 'react';
import { bus } from '@/data/bus';
import { useChainStore } from '@/data/store';
import type { ChainBlock } from '@/data/types';
import { drawBlockBar, drawBlockBarHighlight, BAR_HEIGHT, BAR_GAP } from './blockBar';
import { GasWaveform } from './GasWaveform';
import { WaterfallTooltip, type TooltipData } from './tooltip';
import { BlockPulse } from './pulse';

/** Configuration constants */
const RING_SIZE = 128; // max blocks in memory
const SCROLL_LERP = 0.12; // smooth scroll interpolation factor
const AUTO_SCROLL_THRESHOLD = 2; // bars from top before auto-scroll re-engages
const NUMBER_WIDTH = 72; // must match blockBar.ts

function WaterfallScene() {
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const containerRef = useRef<HTMLDivElement>(null);

  // Mutable state refs (not React state -- avoid re-renders in animation loop)
  const blocksRef = useRef<ChainBlock[]>([]);
  const scrollOffsetRef = useRef(0); // current smooth scroll Y
  const targetScrollRef = useRef(0); // target scroll Y (jumps on new block)
  const autoScrollRef = useRef(true); // true = lock to newest
  const rafIdRef = useRef(0);
  const tooltipSetRef = useRef<(d: TooltipData | null) => void>(() => {});
  const hoveredIndexRef = useRef(-1);

  // Device pixel ratio for retina
  const dprRef = useRef(Math.min(window.devicePixelRatio || 1, 2));

  /** Resize canvas to match container, accounting for DPR */
  const resizeCanvas = useCallback(() => {
    const canvas = canvasRef.current;
    const container = containerRef.current;
    if (!canvas || !container) return;

    const dpr = dprRef.current;
    const rect = container.getBoundingClientRect();

    canvas.width = rect.width * dpr;
    canvas.height = rect.height * dpr;
    canvas.style.width = `${rect.width}px`;
    canvas.style.height = `${rect.height}px`;

    const ctx = canvas.getContext('2d');
    if (ctx) ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
  }, []);

  /** Main render loop */
  const render = useCallback(() => {
    const canvas = canvasRef.current;
    if (!canvas) return;

    // Skip frame if tab not visible
    if (document.hidden) {
      rafIdRef.current = requestAnimationFrame(render);
      return;
    }

    const ctx = canvas.getContext('2d');
    if (!ctx) return;

    const dpr = dprRef.current;
    const width = canvas.width / dpr;
    const height = canvas.height / dpr;
    const blocks = blocksRef.current;

    // Smooth scroll interpolation
    const currentScroll = scrollOffsetRef.current;
    const targetScroll = targetScrollRef.current;
    scrollOffsetRef.current = currentScroll + (targetScroll - currentScroll) * SCROLL_LERP;

    // Clear
    ctx.clearRect(0, 0, width, height);

    if (blocks.length === 0) {
      // Empty state
      ctx.font = '12px "JetBrains Mono", monospace';
      ctx.textAlign = 'center';
      ctx.textBaseline = 'middle';
      ctx.fillStyle = '#6a5a68'; // --rd-text-dim
      ctx.fillText('Waiting for blocks...', width / 2, height / 2);
      rafIdRef.current = requestAnimationFrame(render);
      return;
    }

    // Determine visible range
    const scrollY = scrollOffsetRef.current;
    const rowHeight = BAR_HEIGHT + BAR_GAP;
    const firstVisible = Math.max(0, Math.floor(scrollY / rowHeight));
    const lastVisible = Math.min(
      blocks.length - 1,
      Math.ceil((scrollY + height) / rowHeight),
    );

    // Draw only visible block bars
    for (let i = firstVisible; i <= lastVisible; i++) {
      const block = blocks[i];
      if (!block) continue;

      const y = i * rowHeight - scrollY;

      // Highlight hovered bar
      if (i === hoveredIndexRef.current) {
        const gasLimit = Number(block.gasLimit);
        const gasUsed = Number(block.gasUsed);
        const gasRatio = gasLimit > 0 ? gasUsed / gasLimit : 0;
        const barAreaWidth = width - NUMBER_WIDTH - 56;
        const barWidth = Math.max(4, gasRatio * barAreaWidth);
        drawBlockBarHighlight(ctx, block, y, width, barWidth);
      }

      drawBlockBar(ctx, block, y, width);
    }

    // Draw scroll indicator (right edge)
    if (blocks.length > 0) {
      const totalHeight = blocks.length * rowHeight;
      const viewRatio = height / totalHeight;
      if (viewRatio < 1) {
        const thumbHeight = Math.max(20, height * viewRatio);
        const thumbY = (scrollY / totalHeight) * height;

        ctx.fillStyle = 'rgba(170, 112, 136, 0.25)'; // --rd-rose at 25%
        ctx.fillRect(width - 4, thumbY, 3, thumbHeight);
      }
    }

    rafIdRef.current = requestAnimationFrame(render);
  }, []);

  /** Handle new block from EventBus */
  const onNewBlock = useCallback((block: ChainBlock) => {
    const blocks = blocksRef.current;

    // Prepend (newest first)
    blocks.unshift(block);

    // Ring buffer: evict oldest if over capacity
    if (blocks.length > RING_SIZE) {
      blocks.pop();
    }

    // Auto-scroll: shift target so newest block is at top
    if (autoScrollRef.current) {
      targetScrollRef.current = 0;
    } else {
      // User is scrolled down -- shift target to keep their position stable
      targetScrollRef.current += BAR_HEIGHT + BAR_GAP;
    }
  }, []);

  /** Mouse wheel / trackpad scroll */
  const onWheel = useCallback((e: WheelEvent) => {
    e.preventDefault();

    const rowHeight = BAR_HEIGHT + BAR_GAP;
    const maxScroll = Math.max(
      0,
      blocksRef.current.length * rowHeight -
        (canvasRef.current?.height ?? 0) / dprRef.current,
    );

    targetScrollRef.current = Math.max(
      0,
      Math.min(maxScroll, targetScrollRef.current + e.deltaY),
    );

    // Re-engage auto-scroll if near top
    if (targetScrollRef.current < AUTO_SCROLL_THRESHOLD * rowHeight) {
      autoScrollRef.current = true;
      targetScrollRef.current = 0;
    } else {
      autoScrollRef.current = false;
    }
  }, []);

  /** Hover detection: find which block bar the mouse is over */
  const onMouseMove = useCallback((e: MouseEvent) => {
    const canvas = canvasRef.current;
    if (!canvas) return;

    const rect = canvas.getBoundingClientRect();
    const y = e.clientY - rect.top;

    const scrollY = scrollOffsetRef.current;
    const rowHeight = BAR_HEIGHT + BAR_GAP;
    const index = Math.floor((y + scrollY) / rowHeight);
    const blocks = blocksRef.current;

    hoveredIndexRef.current = index;

    if (index >= 0 && index < blocks.length) {
      const block = blocks[index];
      tooltipSetRef.current({
        block,
        x: e.clientX,
        y: e.clientY,
        barY: index * rowHeight - scrollY,
      });
    } else {
      hoveredIndexRef.current = -1;
      tooltipSetRef.current(null);
    }
  }, []);

  /** Click: emit entity:select for BlockDetail navigation */
  const onClick = useCallback((e: MouseEvent) => {
    const canvas = canvasRef.current;
    if (!canvas) return;

    const rect = canvas.getBoundingClientRect();
    const y = e.clientY - rect.top;

    const scrollY = scrollOffsetRef.current;
    const rowHeight = BAR_HEIGHT + BAR_GAP;
    const index = Math.floor((y + scrollY) / rowHeight);
    const blocks = blocksRef.current;

    if (index >= 0 && index < blocks.length) {
      bus.emit('entity:select', {
        type: 'block',
        id: blocks[index].number.toString(),
      });
    }
  }, []);

  const onMouseLeave = useCallback(() => {
    hoveredIndexRef.current = -1;
    tooltipSetRef.current(null);
  }, []);

  // Touch support
  const touchStartRef = useRef(0);
  const touchScrollStartRef = useRef(0);

  const onTouchStart = useCallback((e: TouchEvent) => {
    touchStartRef.current = e.touches[0].clientY;
    touchScrollStartRef.current = targetScrollRef.current;
  }, []);

  const onTouchMove = useCallback((e: TouchEvent) => {
    e.preventDefault();
    const deltaY = touchStartRef.current - e.touches[0].clientY;
    const rowHeight = BAR_HEIGHT + BAR_GAP;
    const maxScroll = Math.max(
      0,
      blocksRef.current.length * rowHeight -
        (canvasRef.current?.height ?? 0) / dprRef.current,
    );

    targetScrollRef.current = Math.max(
      0,
      Math.min(maxScroll, touchScrollStartRef.current + deltaY),
    );

    autoScrollRef.current =
      targetScrollRef.current < AUTO_SCROLL_THRESHOLD * rowHeight;
  }, []);

  // Lifecycle
  useEffect(() => {
    resizeCanvas();

    const canvas = canvasRef.current;
    if (canvas) {
      canvas.addEventListener('wheel', onWheel, { passive: false });
      canvas.addEventListener('mousemove', onMouseMove);
      canvas.addEventListener('click', onClick);
      canvas.addEventListener('mouseleave', onMouseLeave);
      canvas.addEventListener('touchstart', onTouchStart, { passive: true });
      canvas.addEventListener('touchmove', onTouchMove, { passive: false });
    }

    // Backfill from chain store (existing blocks)
    const storeBlocks = useChainStore.getState().blocks;
    const existing: ChainBlock[] = [];
    for (const block of storeBlocks.values()) {
      existing.push(block);
      if (existing.length >= RING_SIZE) break;
    }
    blocksRef.current = existing;

    // Subscribe to new blocks
    bus.on('block:new', onNewBlock);

    // Resize observer
    const observer = new ResizeObserver(resizeCanvas);
    if (containerRef.current) observer.observe(containerRef.current);

    // Start render loop
    rafIdRef.current = requestAnimationFrame(render);

    return () => {
      cancelAnimationFrame(rafIdRef.current);
      bus.off('block:new', onNewBlock);
      observer.disconnect();

      if (canvas) {
        canvas.removeEventListener('wheel', onWheel);
        canvas.removeEventListener('mousemove', onMouseMove);
        canvas.removeEventListener('click', onClick);
        canvas.removeEventListener('mouseleave', onMouseLeave);
        canvas.removeEventListener('touchstart', onTouchStart);
        canvas.removeEventListener('touchmove', onTouchMove);
      }
    };
  }, [resizeCanvas, render, onNewBlock, onWheel, onMouseMove, onClick, onMouseLeave, onTouchStart, onTouchMove]);

  return (
    <div style={{ display: 'flex', flexDirection: 'column', height: '100%' }}>
      <div
        ref={containerRef}
        style={{
          width: '100%',
          flex: 1,
          minHeight: 0,
          position: 'relative',
          overflow: 'hidden',
        }}
      >
        <canvas
          ref={canvasRef}
          style={{
            display: 'block',
            width: '100%',
            height: '100%',
            cursor: 'pointer',
          }}
        />
        <WaterfallTooltip setRef={tooltipSetRef} />
      </div>
      <BlockPulse />
      <GasWaveform />
    </div>
  );
}

export default WaterfallScene;
