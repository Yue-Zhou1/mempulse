import { useCallback, useEffect, useMemo, useRef } from 'react';
import TextTransition from 'react-text-transition';
import {
  buildRollingDigitSlots,
  resolveRollingTransitionDirection,
  resolveRollingTransitionSpringConfig,
} from '../lib/rolling-number-transition.js';

const INT_NUMBER_FORMATTER = new Intl.NumberFormat();

function useRollingNumberTransition(value, formatValue, options = {}) {
  const durationMs = Number.isFinite(options.durationMs) ? options.durationMs : 520;
  const sanitizedTarget = Number.isFinite(value) ? value : 0;
  const text = useMemo(
    () => formatValue(sanitizedTarget),
    [formatValue, sanitizedTarget],
  );
  const previousTextRef = useRef(text);
  const previousValueRef = useRef(sanitizedTarget);
  const previousValue = previousValueRef.current;
  const direction = resolveRollingTransitionDirection(previousValue, sanitizedTarget);
  const springConfig = useMemo(
    () => resolveRollingTransitionSpringConfig(durationMs),
    [durationMs],
  );
  const slots = useMemo(
    () => buildRollingDigitSlots(previousTextRef.current, text),
    [text],
  );
  useEffect(() => {
    previousValueRef.current = sanitizedTarget;
    previousTextRef.current = text;
  }, [sanitizedTarget, text]);

  return {
    text,
    slots,
    direction,
    springConfig,
  };
}

function RollingDigitSlots({ slots, direction, springConfig }) {
  return (
    <span className="inline-flex items-baseline tabular-nums" aria-hidden="true">
      {slots.map((slot) => (
        slot.isDigit
          ? (
              <span key={slot.key} className="inline-flex w-[1ch] justify-center overflow-hidden">
                <TextTransition inline direction={direction} springConfig={springConfig}>
                  {slot.char}
                </TextTransition>
              </span>
            )
          : (
              <span key={slot.key} className="inline-flex min-w-[0.35ch] justify-center">
                {slot.char}
              </span>
            )
      ))}
    </span>
  );
}

export function RollingInt({ value, durationMs = 520, className }) {
  const formatValue = useCallback(
    (nextValue) => INT_NUMBER_FORMATTER.format(Math.round(Number(nextValue) || 0)),
    [],
  );
  const { text, slots, direction, springConfig } = useRollingNumberTransition(value, formatValue, {
    durationMs,
  });
  return (
    <span className={className} aria-label={text}>
      <RollingDigitSlots slots={slots} direction={direction} springConfig={springConfig} />
    </span>
  );
}

export function RollingPercent({ value, durationMs = 520, className, suffix = '%' }) {
  const formatValue = useCallback(
    (nextValue) => `${(Number(nextValue) || 0).toFixed(1)}${suffix}`,
    [suffix],
  );
  const { text, slots, direction, springConfig } = useRollingNumberTransition(value, formatValue, {
    durationMs,
  });
  return (
    <span className={className} aria-label={text}>
      <RollingDigitSlots slots={slots} direction={direction} springConfig={springConfig} />
    </span>
  );
}
