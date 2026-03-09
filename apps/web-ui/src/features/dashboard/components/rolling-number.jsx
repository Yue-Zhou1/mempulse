import { useCallback, useEffect, useLayoutEffect, useRef } from 'react';
import { createRollingNumberAnimator } from '../lib/rolling-number-animator.js';

const INT_NUMBER_FORMATTER = new Intl.NumberFormat();

function useAnimatedSpan(targetValue, durationMs, format) {
  const spanRef = useRef(null);
  const animatorRef = useRef(null);
  const formatRef = useRef(format);
  formatRef.current = format;

  if (!animatorRef.current) {
    animatorRef.current = createRollingNumberAnimator();
  }

  // Initialize span content and animator state synchronously before first paint
  useLayoutEffect(() => {
    if (spanRef.current) {
      spanRef.current.textContent = formatRef.current(targetValue);
    }
    animatorRef.current.start({
      fromValue: targetValue,
      toValue: targetValue,
      durationMs: 0,
    });
    return () => animatorRef.current?.stop();
  }, []); // eslint-disable-line react-hooks/exhaustive-deps

  // Animate to new target on value change
  useEffect(() => {
    const span = spanRef.current;
    if (!span) return;
    animatorRef.current.retarget({
      toValue: targetValue,
      durationMs,
      onUpdate: (v) => { span.textContent = formatRef.current(v); },
      onComplete: (v) => { span.textContent = formatRef.current(v); },
    });
  }, [targetValue, durationMs]);

  return spanRef;
}

export function RollingInt({ value, durationMs = 520, className }) {
  const sanitized = Number.isFinite(value) ? value : 0;
  const format = useCallback((v) => INT_NUMBER_FORMATTER.format(Math.round(v)), []);
  const spanRef = useAnimatedSpan(sanitized, durationMs, format);
  // High-water-mark: reserve width for the widest value seen so far.
  // Updated during render (safe — Math.max is idempotent, ref mutation doesn't trigger re-render).
  const maxCharsRef = useRef(0);
  const targetText = format(sanitized);
  maxCharsRef.current = Math.max(maxCharsRef.current, targetText.length);
  return (
    <span
      className={className}
      aria-label={targetText}
      style={{ minWidth: `${maxCharsRef.current}ch`, fontFamily: 'var(--news-mono-font)' }}
    >
      <span ref={spanRef} aria-hidden="true" className="tabular-nums" />
    </span>
  );
}

export function RollingPercent({ value, durationMs = 520, className, suffix = '%' }) {
  const sanitized = Number.isFinite(value) ? value : 0;
  const format = useCallback((v) => `${(v || 0).toFixed(1)}${suffix}`, [suffix]);
  const spanRef = useAnimatedSpan(sanitized, durationMs, format);
  const maxCharsRef = useRef(0);
  const targetText = format(sanitized);
  maxCharsRef.current = Math.max(maxCharsRef.current, targetText.length);
  return (
    <span
      className={className}
      aria-label={targetText}
      style={{ minWidth: `${maxCharsRef.current}ch`, fontFamily: 'var(--news-mono-font)' }}
    >
      <span ref={spanRef} aria-hidden="true" className="tabular-nums" />
    </span>
  );
}
