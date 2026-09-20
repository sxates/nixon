'use client';

/**
 * The "Show advanced options" switch (specs/0067 W0).
 *
 * Nixon asks users to make two expert choices it can make for them: which transcription
 * engine (two engines, thirteen models between them) and which summary model (four
 * built-ins whose names — "Qwen 3.5 2B", "gemma3:4b" — mean nothing to anyone who is not
 * already an AI hobbyist). Both were already *computed*: the summary recommendation comes
 * from the machine's RAM and is written as the default on first run, and the transcription
 * engine has a documented "(recommended)" option. The menus were asking a question whose
 * answer the app already had.
 *
 * One switch rather than a disclosure triangle per section, so the concept is learned once
 * and applies everywhere. Off by default; the sections it governs each fall back to a
 * single resolved row. See `isAdvancedRowVisible` for the rule that keeps a deliberately
 * chosen non-default setting on screen regardless.
 */

import { Switch } from '@/components/ui/switch';
import { useConfig } from '@/contexts/ConfigContext';
import { SettingsGroup, SettingsRow, SettingsSection } from '@/components/ui/settings';

export function AdvancedOptionsSettings() {
  const { showAdvanced, toggleShowAdvanced } = useConfig();

  return (
    <SettingsSection
      title="Advanced"
      description="Nixon picks its transcription and summary models to suit this Mac. Turn this on to choose them yourself."
    >
      <SettingsGroup>
        <SettingsRow
          label="Show advanced options"
          description="Adds engine and model pickers to the Transcription and Summary tabs, including cloud providers."
          control={
            <Switch
              checked={showAdvanced}
              onCheckedChange={toggleShowAdvanced}
              aria-label="Show advanced options"
            />
          }
        />
      </SettingsGroup>
    </SettingsSection>
  );
}

/**
 * Whether a section should show its expert controls.
 *
 * The switch is not the only reason to show them: a setting someone deliberately moved off
 * the recommended value has to stay visible, or turning the switch off would hide a choice
 * they are actively relying on — and leave them no way to find or undo it. So the controls
 * appear when advanced mode is on **or** when the current value is not the recommended one.
 *
 * Exported as a plain function so both tabs share one rule and it can be tested without
 * rendering either of them.
 */
export function isAdvancedRowVisible({
  showAdvanced,
  isRecommended,
}: {
  showAdvanced: boolean;
  /** False when the user is running something other than what Nixon would have picked. */
  isRecommended: boolean;
}): boolean {
  return showAdvanced || !isRecommended;
}
