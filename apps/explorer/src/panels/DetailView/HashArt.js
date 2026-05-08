import { jsx as _jsx } from "react/jsx-runtime";
import { useEffect, useRef, memo } from 'react';
import { generateHashArt } from '@/lib/hashArt';
export const HashArt = memo(function HashArt({ hash, size = 64, className, onClick, }) {
    const canvasRef = useRef(null);
    useEffect(() => {
        const canvas = canvasRef.current;
        if (!canvas)
            return;
        const ctx = canvas.getContext('2d');
        if (!ctx)
            return;
        generateHashArt(ctx, hash, size);
    }, [hash, size]);
    return (_jsx("canvas", { ref: canvasRef, width: size, height: size, className: className, onClick: onClick, style: {
            width: size,
            height: size,
            imageRendering: 'pixelated',
            cursor: onClick ? 'pointer' : 'default',
        }, "aria-label": `Generative art for hash ${hash}`, role: "img" }));
});
