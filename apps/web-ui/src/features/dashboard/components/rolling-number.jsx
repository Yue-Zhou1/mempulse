import { useCallback, useEffect, useMemo, useRef } from 'react';
import { createRollingNumberAnimator } from '../lib/rolling-number-animator.js';

const INT_NUMBER_FORMATTER = new Intl.NumberFormat();

function useRollingNumber(value, formatValue, options = {}) {
  const durationMs = Number.isFinite(options.durationMs)
    ? Math.max(120, Math.floor(options.durationMs))
    : 520;
  const sanitizedTarget = Number.isFinite(value) ? value : 0;
  const displayValueRef = useRef(sanitizedTarget);
  const spanRef = useRef(null);
  const animatorRef = useRef(null);
  const stopAnimationRef = useRef(null);
  const lastRenderedTextRef = useRef(formatValue(sanitizedTarget));

  if (!animatorRef.current) {
    animatorRef.current = createRollingNumberAnimator();
  }

  useEffect(() => {
    const node = spanRef.current;
    if (!node) {
      return undefined;
    }

    const updateDisplay = (nextValue) => {
      displayValueRef.current = nextValue;
      const text = formatValue(nextValue);
      if (text === lastRenderedTextRef.current) {
        return;
      }
      node.textContent = text;
      lastRenderedTextRef.current = text;
    };

    const fromValue = displayValueRef.current;
    if (sanitizedTarget <= fromValue) {
      if (typeof stopAnimationRef.current === 'function') {
        stopAnimationRef.current();
      }
      updateDisplay(sanitizedTarget);
      return undefined;
    }

    if (typeof stopAnimationRef.current === 'function') {
      stopAnimationRef.current();
    }
    stopAnimationRef.current = animatorRef.current.start({
      fromValue,
      toValue: sanitizedTarget,
      durationMs,
      onUpdate: updateDisplay,
      onComplete: updateDisplay,
    });

    return () => {
      if (typeof stopAnimationRef.current === 'function') {
        stopAnimationRef.current();
        stopAnimationRef.current = null;
      }
    };
  }, [durationMs, formatValue, sanitizedTarget]);

  const initialText = useMemo(
    () => formatValue(sanitizedTarget),
    [formatValue, sanitizedTarget],
  );

  return {
    spanRef,
    initialText,
  };
}

export function RollingInt({ value, durationMs = 520, className }) {
  const formatValue = useCallback(
    (nextValue) => INT_NUMBER_FORMATTER.format(Math.round(Number(nextValue) || 0)),
    [],
  );
  const { spanRef, initialText } = useRollingNumber(value, formatValue, { durationMs });
  return (
    <span ref={spanRef} className={className}>
      {initialText}
    </span>
  );
}

export function RollingPercent({ value, durationMs = 520, className, suffix = '%' }) {
  const formatValue = useCallback(
    (nextValue) => `${(Number(nextValue) || 0).toFixed(1)}${suffix}`,
    [suffix],
  );
  const { spanRef, initialText } = useRollingNumber(value, formatValue, { durationMs });
  return (
    <span ref={spanRef} className={className}>
      {initialText}
    </span>
  );
}
