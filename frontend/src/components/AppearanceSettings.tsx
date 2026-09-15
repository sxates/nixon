'use client';

import React from 'react';
import { useTheme, type ThemePreference } from '@/contexts/ThemeContext';
import { cn } from '@/lib/utils';
import { SettingsGroup, SettingsRow, SettingsSection } from '@/components/ui/settings';

// specs/0057 decision 1 — Faceplate (light) / Deck (dark) / System. Rendered as a
// three-position selector, not a switch: a deck has a labelled position for each state.
const OPTIONS: Array<{ value: ThemePreference; label: string; hint: string }> = [
  { value: 'light', label: 'Faceplate', hint: 'Brushed aluminum, ink labels' },
  { value: 'dark', label: 'Deck', hint: 'Charcoal chassis, cream silkscreen' },
  { value: 'system', label: 'System', hint: 'Follow the macOS appearance' },
];

export function AppearanceSettings() {
  const { preference, setPreference } = useTheme();
  const buttonRefs = React.useRef<Array<HTMLButtonElement | null>>([]);
  const selectedIndex = Math.max(
    0,
    OPTIONS.findIndex((o) => o.value === preference),
  );

  // WAI-ARIA radiogroup keyboard contract: the group is ONE tab stop (roving tabIndex) and
  // the arrows move — and select — within it, wrapping at both ends. Without this the three
  // positions were three tab stops and unreachable by arrow, which is how a native radio
  // group behaves for keyboard/VoiceOver users.
  const move = (delta: number) => {
    const next = (selectedIndex + delta + OPTIONS.length) % OPTIONS.length;
    setPreference(OPTIONS[next].value);
    buttonRefs.current[next]?.focus();
  };

  const handleKeyDown = (event: React.KeyboardEvent<HTMLDivElement>) => {
    if (event.key === 'ArrowLeft' || event.key === 'ArrowUp') {
      event.preventDefault();
      move(-1);
    } else if (event.key === 'ArrowRight' || event.key === 'ArrowDown') {
      event.preventDefault();
      move(1);
    }
  };

  return (
    <SettingsSection
      id="appearance-label"
      title="Appearance"
      description="Which face the machine wears. System follows macOS."
    >
      <SettingsGroup>
        <SettingsRow label="Theme" align="start">
          <div
            role="radiogroup"
            aria-labelledby="appearance-label"
            onKeyDown={handleKeyDown}
            className="grid grid-cols-3 gap-2"
          >
            {OPTIONS.map((opt, index) => {
              const selected = preference === opt.value;
              return (
                <button
                  key={opt.value}
                  type="button"
                  role="radio"
                  aria-checked={selected}
                  tabIndex={index === selectedIndex ? 0 : -1}
                  ref={(el) => {
                    buttonRefs.current[index] = el;
                  }}
                  onClick={() => setPreference(opt.value)}
                  className={cn(
                    'flex flex-col items-start gap-1 rounded-md border px-3 py-2 text-left transition-colors',
                    selected
                      ? 'border-brand bg-card text-foreground'
                      : 'border-border bg-panel text-muted-foreground hover:bg-accent',
                  )}
                >
                  <span className="flex items-center gap-2 text-sm font-semibold">
                    <span
                      aria-hidden
                      className={cn('inline-block h-2 w-2 rounded-full', selected ? 'bg-brand' : 'bg-border')}
                    />
                    {opt.label}
                  </span>
                  <span className="text-xs">{opt.hint}</span>
                </button>
              );
            })}
          </div>
        </SettingsRow>
      </SettingsGroup>
    </SettingsSection>
  );
}
