'use client';

import { useTheme, type ThemePreference } from '@/contexts/ThemeContext';
import { OptionCard, OptionCardGroup } from '@/components/ui/option-card';
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

  // OptionCardGroup is the WAI-ARIA radiogroup: ONE tab stop (roving tabIndex) and the
  // arrows move — and select — within it, wrapping at both ends.
  return (
    <SettingsSection
      id="appearance-label"
      title="Appearance"
      description="Which face the machine wears. System follows macOS."
    >
      <SettingsGroup>
        <SettingsRow label="Theme" align="start">
          <OptionCardGroup
            value={preference}
            onValueChange={(value) => setPreference(value as ThemePreference)}
            aria-labelledby="appearance-label"
            className="grid grid-cols-3 gap-2"
          >
            {OPTIONS.map((opt) => (
              <OptionCard key={opt.value} value={opt.value} title={opt.label} description={opt.hint} />
            ))}
          </OptionCardGroup>
        </SettingsRow>
      </SettingsGroup>
    </SettingsSection>
  );
}
