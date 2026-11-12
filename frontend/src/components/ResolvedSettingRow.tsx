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
import { cn } from '@/lib/utils';
import { Button } from '@/components/ui/button';
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
              <span className="u-section-label" data-testid="resolved-value">
                {value}
              </span>
              <Button variant="outline" size="sm" aria-expanded={false} onClick={() => setExpanded(true)}>
                Change
              </Button>
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
