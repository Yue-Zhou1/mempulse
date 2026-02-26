import { useEffect, useRef } from 'react';
import { useInView, useMotionValue, useSpring } from 'motion/react';
import { cn } from '../../lib/utils.js';

export function NumberTicker({
  value,
  startValue = 0,
  direction = 'up',
  delay = 0,
  className,
  decimalPlaces = 0,
  ...props
}) {
  const ref = useRef(null);
  const motionValue = useMotionValue(direction === 'down' ? value : startValue);
  const springValue = useSpring(motionValue, {
    damping: 60,
    stiffness: 100,
  });
  const isInView = useInView(ref, { once: true, margin: '0px' });

  useEffect(() => {
    if (isInView) {
      const timer = setTimeout(() => {
        motionValue.set(direction === 'down' ? startValue : value);
      }, delay * 1000);
      return () => clearTimeout(timer);
    }
    return undefined;
  }, [motionValue, isInView, delay, value, direction, startValue]);

  useEffect(() => {
    return springValue.on('change', (latest) => {
      if (ref.current) {
        ref.current.textContent = Intl.NumberFormat('en-US', {
          minimumFractionDigits: decimalPlaces,
          maximumFractionDigits: decimalPlaces,
        }).format(Number(latest.toFixed(decimalPlaces)));
      }
    });
  }, [springValue, decimalPlaces]);

  return (
    <span
      ref={ref}
      className={cn('inline-block tabular-nums tracking-tight', className)}
      {...props}
    >
      {startValue}
    </span>
  );
}
