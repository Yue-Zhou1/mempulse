import { useEffect, useRef, useState } from 'react';

function easeOutCubic(progress) {
  return 1 - (1 - progress) ** 3;
}

function useRollingNumber(value, options = {}) {
  const durationMs = Number.isFinite(options.durationMs)
    ? Math.max(120, Math.floor(options.durationMs))
    : 520;
  const sanitizedTarget = Number.isFinite(value) ? value : 0;
  const [displayValue, setDisplayValue] = useState(sanitizedTarget);
  const displayValueRef = useRef(sanitizedTarget);
  const frameRef = useRef(null);

  useEffect(() => {
    displayValueRef.current = displayValue;
  }, [displayValue]);

  useEffect(() => {
    const fromValue = displayValueRef.current;
    if (sanitizedTarget <= fromValue) {
      if (frameRef.current) {
        window.cancelAnimationFrame(frameRef.current);
        frameRef.current = null;
      }
      displayValueRef.current = sanitizedTarget;
      setDisplayValue(sanitizedTarget);
      return undefined;
    }

    let startTs = null;
    const delta = sanitizedTarget - fromValue;

    const tick = (ts) => {
      if (startTs == null) {
        startTs = ts;
      }
      const progress = Math.min(1, (ts - startTs) / durationMs);
      const nextValue = fromValue + delta * easeOutCubic(progress);
      displayValueRef.current = nextValue;
      setDisplayValue(nextValue);
      if (progress < 1) {
        frameRef.current = window.requestAnimationFrame(tick);
      } else {
        frameRef.current = null;
      }
    };

    frameRef.current = window.requestAnimationFrame(tick);
    return () => {
      if (frameRef.current) {
        window.cancelAnimationFrame(frameRef.current);
        frameRef.current = null;
      }
    };
  }, [durationMs, sanitizedTarget]);

  return displayValue;
}

export function RollingInt({ value, durationMs = 520, className }) {
  const display = useRollingNumber(value, { durationMs });
  return <span className={className}>{Math.round(display).toLocaleString()}</span>;
}

export function RollingPercent({ value, durationMs = 520, className, suffix = '%' }) {
  const display = useRollingNumber(value, { durationMs });
  return (
    <span className={className}>
      {display.toFixed(1)}
      {suffix}
    </span>
  );
}
