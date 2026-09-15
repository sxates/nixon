'use client';

import { useState } from 'react';
import { Pin } from 'lucide-react';
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
                  className={`inline-flex items-center rounded-[3px] border text-sm overflow-hidden ${
                    isPinned
                      ? 'bg-brand/10 border-brand/20 text-brand'
                      : 'bg-muted border-border text-foreground'
                  }`}
                >
                  <button
                    type="button"
                    aria-label={isPinned ? `Unpin ${labelForCode(code)} as default` : `Pin ${labelForCode(code)} as default`}
                    aria-pressed={isPinned}
                    title={isPinned ? 'Click to unset as default' : 'Click to set as default'}
                    onClick={() => togglePin(code)}
                    className={`flex items-center gap-1.5 pl-3 pr-2 py-1 hover:brightness-95 active:brightness-90 ${
                      isPinned ? 'text-brand' : 'text-foreground'
                    }`}
                  >
                    <Pin
                      size={14}
                      className={isPinned ? 'text-brand' : 'text-muted-foreground'}
                      fill={isPinned ? 'currentColor' : 'none'}
                    />
                    {labelForCode(code)}
                  </button>
                  <button
                    type="button"
                    aria-label={`Remove ${labelForCode(code)}`}
                    onClick={() => removeRecent(code)}
                    className={`pr-2.5 pl-0.5 py-1 leading-none ${isPinned ? 'text-brand/70 hover:text-brand' : 'text-muted-foreground hover:text-foreground'}`}
                  >
                    ×
                  </button>
                </span>
              );
            })}

            <Popover open={pickerOpen} onOpenChange={setPickerOpen}>
              <PopoverTrigger asChild>
                <button
                  type="button"
                  disabled={recents.length >= 5}
                  className="inline-flex items-center gap-1 rounded-[3px] border border-dashed border-input px-3 py-1 text-sm text-muted-foreground hover:border-border hover:text-foreground disabled:cursor-not-allowed disabled:opacity-50"
                >
                  + Add language
                </button>
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
