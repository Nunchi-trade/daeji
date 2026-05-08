import { useState, useEffect, useRef } from 'react';

/**
 * Tracks frame rate using requestAnimationFrame.
 * Returns the current FPS as a rounded integer.
 * Only updates the React state once per second to avoid thrashing.
 */
export function useFrameMonitor(): number {
  const [fps, setFps] = useState(60);
  const framesRef = useRef(0);
  const lastRef = useRef(performance.now());
  const rafRef = useRef<number>(0);

  useEffect(() => {
    // Only run in dev mode
    if (import.meta.env.PROD) return;

    const tick = (now: number) => {
      framesRef.current++;

      const elapsed = now - lastRef.current;
      if (elapsed >= 1000) {
        setFps(Math.round((framesRef.current * 1000) / elapsed));
        framesRef.current = 0;
        lastRef.current = now;
      }

      rafRef.current = requestAnimationFrame(tick);
    };

    rafRef.current = requestAnimationFrame(tick);
    return () => {
      if (rafRef.current) cancelAnimationFrame(rafRef.current);
    };
  }, []);

  return fps;
}
