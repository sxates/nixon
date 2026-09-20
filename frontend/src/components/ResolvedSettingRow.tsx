'use client';

/**
 * A setting Nixon decided for you: what is in use, and a way to change it (specs/0067).
 *
 * This replaces the global "Show advanced options" switch, which shipped and was rejected
 * in review for the right reason — a control on one tab changed what other tabs showed, so
 * its effect was invisible from where you operated it. The affordance belongs where the
 * setting is: this row states the resolved value and expands the real controls in place.
 *
 * The rule that survives from the switch: a value that is NOT the recommended one renders
 * expanded from the start and cannot be collapsed away, because someone who deliberately
 * chose a cloud provider or a different model must keep seeing it. That is `locked` below.
 */

import { useState, type ReactNode } from 'react';
import { ChevronDown } from 'lucide-react';
import { cn } from '@/lib/utils';
import { SettingsGroup, SettingsRow } from '@/components/ui/settings';

export function ResolvedSettingRow({
  label,
  description,
  value,
  /** True when the current value is not what Nixon would pick: show the controls, always. */
  locked = false,
  children,
}: {
  label: string;
  description: ReactNode;
  value: ReactNode;
  locked?: boolean;
  children: ReactNode;
}) {
  const [expanded, setExpanded] = useState(false);
  const open = locked || expanded;

  return (
    <SettingsGroup>
      {!open && (
        <SettingsRow
          label={label}
          description={description}
          control={
            <div className="flex items-center gap-3">
              <span className="u-section-label text-[11px] text-engrave" data-testid="resolved-value">
                {value}
              </span>
              <button
                type="button"
                onClick={() => setExpanded(true)}
                className="u-section-label flex items-center gap-1 rounded-[3px] border border-border px-2 py-0.5 text-[9px] text-muted-foreground transition-colors hover:bg-key hover:text-foreground focus:outline-none focus-visible:ring-2 focus-visible:ring-ring"
              >
                Change
                <ChevronDown className="h-3 w-3" aria-hidden />
              </button>
            </div>
          }
        />
      )}
      {open && (
        <div className={cn('py-4', locked && 'pt-0')}>
          {children}
        </div>
      )}
    </SettingsGroup>
  );
}
