import { useMotionValue, animate, useTransform, motion } from 'framer-motion';
import { useEffect } from 'react';

interface AnimatedNumberProps {
  value: number;
  format?: (n: number) => string;
  duration?: number;
  style?: React.CSSProperties;
  className?: string;
}

export function AnimatedNumber({
  value,
  format = (n) => Math.round(n).toLocaleString(),
  duration = 0.6,
  style,
  className,
}: AnimatedNumberProps) {
  const motionValue = useMotionValue(0);
  const formatted = useTransform(motionValue, format);

  useEffect(() => {
    const controls = animate(motionValue, value, {
      duration,
      ease: [0.16, 1, 0.3, 1],
    });
    return controls.stop;
  }, [value, duration, motionValue]);

  return (
    <motion.span
      style={{ fontVariantNumeric: 'tabular-nums', ...style }}
      className={className}
    >
      {formatted}
    </motion.span>
  );
}
