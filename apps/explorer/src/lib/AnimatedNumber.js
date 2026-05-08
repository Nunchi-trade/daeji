import { jsx as _jsx } from "react/jsx-runtime";
import { useMotionValue, animate, useTransform, motion } from 'framer-motion';
import { useEffect } from 'react';
export function AnimatedNumber({ value, format = (n) => Math.round(n).toLocaleString(), duration = 0.6, style, className, }) {
    const motionValue = useMotionValue(0);
    const formatted = useTransform(motionValue, format);
    useEffect(() => {
        const controls = animate(motionValue, value, {
            duration,
            ease: [0.16, 1, 0.3, 1],
        });
        return controls.stop;
    }, [value, duration, motionValue]);
    return (_jsx(motion.span, { style: { fontVariantNumeric: 'tabular-nums', ...style }, className: className, children: formatted }));
}
