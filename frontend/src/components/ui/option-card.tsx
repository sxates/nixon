'use client';

/**
 * OptionCard and OptionCardGroup (docs/design-system.md §2) — a selectable card with radio
 * semantics: the theme choice, the Whisper/Parakeet model pickers.
 *
 * Drawn from AppearanceSettings' theme cards: `rounded-md border px-3 py-2`; selected is a
 * brand edge on the card face with a lit brand dot, unselected is the panel face with an
 * unlit dot. `title` is `text-sm font-semibold`, `description` `text-xs`; `trailing` holds
 * a size, a Badge or a download control on the right of the title line.
 *
 * Inside an `OptionCardGroup` the cards take `selected`/`onSelect` from the group, and the
 * group is ONE Tab stop with arrow-key selection (roving tabindex, like `SegmentedControl`).
 * A lone `OptionCard` is controlled by its own `selected`/`onSelect`.
 *
 * Don't nest buttons in `trailing`: the card itself is the button. A card with its own
 * actions (download, delete) needs those outside the card.
 */

import * as React from 'react';
import { StatusDot } from '@/components/ui/status-dot';
import { cn } from '@/lib/utils';

interface GroupContextValue {
  value: string | null;
  select: (value: string) => void;
  /** The value that owns the Tab stop (the selected one, else the first enabled one). */
  tabStop: string | null;
  register: (value: string, el: HTMLButtonElement | null) => void;
}

const GroupContext = React.createContext<GroupContextValue | null>(null);

export interface OptionCardProps
  extends Omit<React.ButtonHTMLAttributes<HTMLButtonElement>, 'title' | 'value' | 'onSelect'> {
  title: React.ReactNode;
  description?: React.ReactNode;
  trailing?: React.ReactNode;
  /** Required inside an OptionCardGroup. */
  value?: string;
  /** Standalone use. Ignored inside a group. */
  selected?: boolean;
  /** Standalone use. Ignored inside a group. */
  onSelect?: () => void;
}

export const OptionCard = React.forwardRef<HTMLButtonElement, OptionCardProps>(function OptionCard(
  { title, description, trailing, value, selected: selectedProp, onSelect, disabled, className, onClick, ...rest },
  ref,
) {
  const group = React.useContext(GroupContext);
  const inGroup = group != null && value != null;
  const selected = inGroup ? group.value === value : !!selectedProp;
  const tabIndex = inGroup ? (group.tabStop === value ? 0 : -1) : undefined;

  // Depends on the group's stable `register`, not the whole context, so a selection
  // change doesn't detach and re-attach every card's ref.
  const register = group?.register;
  const setRef = React.useCallback(
    (el: HTMLButtonElement | null) => {
      if (register && value != null) register(value, el);
      if (typeof ref === 'function') ref(el);
      else if (ref) ref.current = el;
    },
    [register, value, ref],
  );

  return (
    <button
      ref={setRef}
      type="button"
      role="radio"
      aria-checked={selected}
      tabIndex={tabIndex}
      disabled={disabled}
      data-selected={selected || undefined}
      onClick={(e) => {
        onClick?.(e);
        if (e.defaultPrevented) return;
        if (inGroup) group.select(value);
        else onSelect?.();
      }}
      className={cn(
        'flex w-full flex-col items-start gap-1 rounded-md border px-3 py-2 text-left transition-colors',
        'focus:outline-none focus-visible:ring-2 focus-visible:ring-ring',
        'disabled:cursor-default disabled:opacity-60',
        selected
          ? 'border-brand bg-card text-foreground'
          : 'border-border bg-panel text-muted-foreground hover:bg-accent',
        className,
      )}
      {...rest}
    >
      <span className="flex w-full items-center gap-2">
        <StatusDot tone={selected ? 'brand' : undefined} className={selected ? undefined : 'bg-border'} />
        <span className="min-w-0 flex-1 text-sm font-semibold">{title}</span>
        {trailing != null ? <span className="flex flex-none items-center gap-2">{trailing}</span> : null}
      </span>
      {description != null ? <span className="text-xs">{description}</span> : null}
    </button>
  );
});

export interface OptionCardGroupProps {
  value: string | null;
  onValueChange: (value: string) => void;
  'aria-label'?: string;
  'aria-labelledby'?: string;
  children: React.ReactNode;
  className?: string;
}

export function OptionCardGroup({
  value,
  onValueChange,
  'aria-label': ariaLabel,
  'aria-labelledby': ariaLabelledby,
  children,
  className,
}: OptionCardGroupProps) {
  const items = React.useRef(new Map<string, HTMLButtonElement>());
  const [firstValue, setFirstValue] = React.useState<string | null>(null);

  /** Enabled cards in DOM order. */
  const ordered = React.useCallback(() => {
    return Array.from(items.current.entries())
      .filter(([, el]) => !el.disabled)
      .sort(([, a], [, b]) => (a.compareDocumentPosition(b) & Node.DOCUMENT_POSITION_FOLLOWING ? -1 : 1));
  }, []);

  const register = React.useCallback(
    (v: string, el: HTMLButtonElement | null) => {
      if (el) items.current.set(v, el);
      else items.current.delete(v);
      const first = ordered()[0]?.[0] ?? null;
      setFirstValue((prev) => (prev === first ? prev : first));
    },
    [ordered],
  );

  const ctx = React.useMemo<GroupContextValue>(() => {
    const valueIsCard = value != null && items.current.has(value);
    return {
      value,
      select: (v) => {
        if (v !== value) onValueChange(v);
      },
      tabStop: valueIsCard ? value : firstValue,
      register,
    };
  }, [value, onValueChange, firstValue, register]);

  const move = (delta: number) => {
    const list = ordered();
    if (list.length === 0) return;
    const at = list.findIndex(([v]) => v === value);
    const from = at === -1 ? (delta > 0 ? -1 : 0) : at;
    const [nextValue, el] = list[(from + delta + list.length) % list.length]!;
    el.focus();
    if (nextValue !== value) onValueChange(nextValue);
  };

  return (
    <GroupContext.Provider value={ctx}>
      <div
        role="radiogroup"
        aria-label={ariaLabel}
        aria-labelledby={ariaLabelledby}
        className={className}
        onKeyDown={(e) => {
          if (e.key === 'ArrowRight' || e.key === 'ArrowDown') {
            e.preventDefault();
            move(1);
          } else if (e.key === 'ArrowLeft' || e.key === 'ArrowUp') {
            e.preventDefault();
            move(-1);
          }
        }}
      >
        {children}
      </div>
    </GroupContext.Provider>
  );
}
