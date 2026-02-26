import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { cn } from '../../../shared/lib/utils.js';

export function NewspaperFilterSelect({
  id,
  value,
  onChange,
  options,
  label = 'Mainnet',
  compact = false,
  ariaLabel,
  className,
}) {
  const containerRef = useRef(null);
  const [isOpen, setIsOpen] = useState(false);
  const selectedOption = useMemo(
    () => options.find((option) => option.value === value) ?? options[0] ?? null,
    [options, value],
  );
  const triggerLabel = selectedOption?.label ?? 'Select Mainnet';

  const menuId = `${id}-menu`;
  const triggerId = `${id}-trigger`;

  const selectOption = useCallback(
    (nextValue) => {
      if (typeof onChange === 'function') {
        onChange({ target: { value: nextValue } });
      }
      setIsOpen(false);
    },
    [onChange],
  );

  const onTriggerClick = useCallback(() => {
    setIsOpen((current) => !current);
  }, []);

  const onMenuClick = useCallback(
    (event) => {
      const node = event.target;
      if (!(node instanceof Element)) {
        return;
      }
      const optionNode = node.closest('[data-option-value]');
      if (!(optionNode instanceof HTMLElement)) {
        return;
      }
      const nextValue = optionNode.getAttribute('data-option-value');
      if (!nextValue) {
        return;
      }
      selectOption(nextValue);
    },
    [selectOption],
  );

  useEffect(() => {
    if (!isOpen) {
      return undefined;
    }

    const onPointerDown = (event) => {
      const node = event.target;
      if (!(node instanceof Node)) {
        return;
      }
      if (!containerRef.current?.contains(node)) {
        setIsOpen(false);
      }
    };

    document.addEventListener('pointerdown', onPointerDown);
    return () => {
      document.removeEventListener('pointerdown', onPointerDown);
    };
  }, [isOpen]);

  useEffect(() => {
    setIsOpen(false);
  }, [value]);

  return (
    <div
      className={cn('news-filter-control', compact ? 'news-filter-control-compact' : '', className)}
      ref={containerRef}
    >
      <span className="news-filter-label news-mono">{label}</span>
      <div className={cn('news-filter-dropdown', isOpen ? 'news-filter-dropdown-active' : '')}>
        <button
          type="button"
          id={triggerId}
          aria-expanded={isOpen}
          aria-haspopup="listbox"
          aria-controls={menuId}
          onClick={onTriggerClick}
          className="news-filter-trigger"
          aria-label={ariaLabel ?? `${label}: ${triggerLabel}`}
        >
          <span className="news-filter-trigger-text">{triggerLabel}</span>
          <span
            className={cn('news-filter-arrow news-mono', isOpen ? 'news-filter-arrow-open' : '')}
            aria-hidden="true"
          >
            ▼
          </span>
        </button>
        <div
          id={menuId}
          role="listbox"
          aria-labelledby={triggerId}
          className="news-filter-menu"
        >
          <ul className="news-filter-list" onClick={onMenuClick}>
            {options.map((option, index) => {
              const isSelected = option.value === value;
              return (
                <li key={option.value} className="news-filter-item">
                  <div
                    className={cn(
                      'news-filter-option',
                      isSelected ? 'news-filter-option-active' : '',
                    )}
                    role="option"
                    aria-selected={isSelected}
                    data-option-value={option.value}
                  >
                    <span>{option.label}</span>
                    {isSelected ? <span className="news-filter-option-mark">•</span> : null}
                  </div>
                  {index < options.length - 1 ? (
                    <div className="news-filter-separator" aria-hidden="true" />
                  ) : null}
                </li>
              );
            })}
          </ul>
          <div className="news-filter-menu-footer news-mono">
            <span>Vol. Mainnet</span>
            <span>{triggerLabel}</span>
          </div>
        </div>
      </div>
    </div>
  );
}
