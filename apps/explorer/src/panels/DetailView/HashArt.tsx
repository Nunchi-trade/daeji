import { useEffect, useRef, memo } from 'react';
import { generateHashArt } from '@/lib/hashArt';

interface HashArtProps {
  hash: `0x${string}`;
  size?: number;
  className?: string;
  onClick?: () => void;
}

export const HashArt = memo(function HashArt({
  hash,
  size = 64,
  className,
  onClick,
}: HashArtProps) {
  const canvasRef = useRef<HTMLCanvasElement>(null);

  useEffect(() => {
    const canvas = canvasRef.current;
    if (!canvas) return;

    const ctx = canvas.getContext('2d');
    if (!ctx) return;

    generateHashArt(ctx, hash, size);
  }, [hash, size]);

  return (
    <canvas
      ref={canvasRef}
      width={size}
      height={size}
      className={className}
      onClick={onClick}
      style={{
        width: size,
        height: size,
        imageRendering: 'pixelated',
        cursor: onClick ? 'pointer' : 'default',
      }}
      aria-label={`Generative art for hash ${hash}`}
      role="img"
    />
  );
});
