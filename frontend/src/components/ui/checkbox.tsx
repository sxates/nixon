'use client';

import * as React from 'react';

import { DeckIcon } from '@/components/ui/deck-icon';
import { Check } from '@/components/ui/icons';
import { cn } from '@/lib/utils';

/**
 * Checkbox (docs/design-system.md §2): one choice among several, or an option inside a form
 * that a button submits. A single on/off setting that applies at once is a `Switch`.
 *
 * A `<button role="checkbox">` (the same shape Radix's checkbox renders, without the
 * dependency): `aria-checked` carries the state, Space toggles, Enter does not (WAI-ARIA).
 * Controlled with `checked`, or uncontrolled with `defaultChecked`. `onClick` still fires
 * first, so a caller that needs the click itself (shift-click range selection) can drive
 * the state from there instead of `onCheckedChange`.
 *
 * `label` renders the box inside a `<label>` with the text beside it; the text names the
 * checkbox. Without `label`, pass `aria-label`.
 */
export interface CheckboxProps
  extends Omit<React.ButtonHTMLAttributes<HTMLButtonElement>, 'onChange' | 'value' | 'type' | 'role'> {
  checked?: boolean;
  defaultChecked?: boolean;
  onCheckedChange?: (checked: boolean) => void;
  /** `sm` 16px (dense lists, the transcript), `default` 18px. */
  size?: 'sm' | 'default';
  /** Visible label beside the box. */
  label?: React.ReactNode;
  /** Classes for the `<label>` wrapper when `label` is set. */
  labelClassName?: string;
}

const SIZE = {
  sm: 'h-4 w-4',
  default: 'h-[18px] w-[18px]',
} as const;

export const Checkbox = React.forwardRef<HTMLButtonElement, CheckboxProps>(function Checkbox(
  {
    checked: checkedProp,
    defaultChecked = false,
    onCheckedChange,
    size = 'default',
    label,
    labelClassName,
    className,
    onClick,
    onKeyDown,
    id: idProp,
    ...props
  },
  ref,
) {
  const [uncontrolled, setUncontrolled] = React.useState(defaultChecked);
  const isControlled = checkedProp !== undefined;
  const checked = isControlled ? checkedProp : uncontrolled;
  const autoId = React.useId();
  const labelId = `${idProp ?? autoId}-label`;

  const box = (
    <button
      ref={ref}
      type="button"
      role="checkbox"
      id={idProp}
      aria-checked={checked}
      aria-labelledby={label != null && !props['aria-label'] ? labelId : undefined}
      data-state={checked ? 'checked' : 'unchecked'}
      onClick={(e) => {
        onClick?.(e);
        const next = !checked;
        if (!isControlled) setUncontrolled(next);
        onCheckedChange?.(next);
      }}
      onKeyDown={(e) => {
        // A checkbox toggles on Space only; Enter would also click a native button.
        if (e.key === 'Enter') e.preventDefault();
        onKeyDown?.(e);
      }}
      className={cn(
        'peer inline-flex flex-shrink-0 items-center justify-center rounded-sm border transition-[color,background-color,border-color,opacity]',
        'focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring',
        'disabled:cursor-not-allowed disabled:opacity-50',
        checked
          ? 'border-brand bg-brand text-brand-foreground'
          : 'border-input bg-transparent hover:border-brand-edge',
        SIZE[size],
        className,
      )}
      {...props}
    >
      {checked ? <DeckIcon icon={Check} size={size === 'sm' ? 12 : 14} /> : null}
    </button>
  );

  if (label == null) return box;
  return (
    <label
      className={cn(
        'inline-flex min-w-0 cursor-pointer items-center gap-2 text-sm text-foreground has-[:disabled]:cursor-not-allowed',
        labelClassName,
      )}
    >
      {box}
      <span id={labelId} className="min-w-0 select-none">
        {label}
      </span>
    </label>
  );
});
Checkbox.displayName = 'Checkbox';
