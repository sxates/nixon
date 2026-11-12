'use client';

import { useState } from 'react';
import { Button } from '@/components/ui/button';
import { DeckIcon } from '@/components/ui/deck-icon';
import { IconButton } from '@/components/ui/icon-button';
import { Star, X } from '@/components/ui/icons';
import { cn } from '@/lib/utils';
import { Popover, PopoverTrigger, PopoverContent } from '@/components/ui/popover';
import { LanguagePickerPopover } from '@/components/LanguagePickerPopover';
import { useRecentLanguages } from '@/hooks/useRecentLanguages';
import { labelForCode } from '@/lib/summary-languages';
import { SettingsGroup, SettingsRow, SettingsSection } from '@/components/ui/settings';

export function SummaryLanguageSettings() {
  const { recents, pinned, addRecent, removeRecent, setPinned } = useRecentLanguages();
  const [pickerOpen, setPickerOpen] = useState(false);

  const togglePin = (code: string) => {
    setPinned(pinned === code ? null : code);
  };

  return (
    <SettingsSection
      title="Summary language"
      description="Auto uses the dominant transcript language."
    >
      <SettingsGroup>
        <SettingsRow
          label="Quick-switch languages"
          description={
            pinned
              ? `Default: ${labelForCode(pinned)} — click it again to unset. Max 5 quick-switch options.`
              : 'Click any language to set it as your default. Max 5 quick-switch options.'
          }
          align="start"
        >
          <div className="flex flex-wrap items-center gap-2">
            {recents.map((code) => {
              const isPinned = pinned === code;
              return (
                <span
                  key={code}
                  className={cn(
                    'inline-flex items-center overflow-hidden rounded-md border',
                    isPinned ? 'border-brand-edge bg-brand-wash' : 'border-border bg-muted',
                  )}
                >
                  <Button
                    variant="ghost"
                    size="xs"
                    aria-label={isPinned ? `Unpin ${labelForCode(code)} as default` : `Pin ${labelForCode(code)} as default`}
                    aria-pressed={isPinned}
                    title={isPinned ? 'Click to unset as default' : 'Click to set as default'}
                    onClick={() => togglePin(code)}
                    className={cn(
                      'rounded-none pl-3 pr-1.5 text-sm font-normal',
                      isPinned ? 'text-brand hover:text-brand' : 'text-foreground',
                    )}
                  >
                    <DeckIcon
                      icon={Star}
                      className={isPinned ? 'fill-current text-brand' : 'text-muted-foreground'}
                    />
                    {labelForCode(code)}
                  </Button>
                  <IconButton
                    variant="ghost"
                    label={`Remove ${labelForCode(code)}`}
                    icon={<DeckIcon icon={X} size={12} />}
                    onClick={() => removeRecent(code)}
                    className={cn('w-6 rounded-none', isPinned && 'text-brand hover:text-brand')}
                  />
                </span>
              );
            })}

            <Popover open={pickerOpen} onOpenChange={setPickerOpen}>
              <PopoverTrigger asChild>
                <Button variant="dashed" size="xs" disabled={recents.length >= 5}>
                  + Add language
                </Button>
              </PopoverTrigger>
              <PopoverContent align="start" className="w-auto p-0 border-0 shadow-none bg-transparent">
                <LanguagePickerPopover
                  mode="settings"
                  value={null}
                  onChange={(code) => {
                    if (code) addRecent(code);
                    setPickerOpen(false);
                  }}
                  onClose={() => setPickerOpen(false)}
                />
              </PopoverContent>
            </Popover>
          </div>
        </SettingsRow>
      </SettingsGroup>
    </SettingsSection>
  );
}
