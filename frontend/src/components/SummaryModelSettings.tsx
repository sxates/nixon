'use client';

/**
 * When Nixon writes a summary (specs/0067).
 *
 * All that is left of what used to be the whole Summary tab: the model moved to its own
 * component so it could sit at the bottom, and the summary-language section was removed —
 * Auto detects the transcript's own language, and the section existed only to curate
 * quick-switch chips for a picker most people never open.
 */

import { Switch } from './ui/switch';
import { useConfig } from '@/contexts/ConfigContext';
import { SettingsGroup, SettingsRow, SettingsSection } from '@/components/ui/settings';

export function SummaryModelSettings() {
  const { isAutoSummary, toggleIsAutoSummary } = useConfig();

  return (
    <div className="space-y-8">
      <SettingsSection
        title="When to summarize"
        description="Summaries can also be generated on demand from any meeting page."
      >
        <SettingsGroup>
          {/* specs/0029 WS7.3. This used to be duplicated as a second switch in Recording
              settings (same ConfigContext state, two surfaces to keep in sync); that copy
              is gone now (specs/0061 W6) and Recording settings just points here. */}
          <SettingsRow
            label="Summarize automatically when a meeting ends"
            description="Generate an AI summary as soon as a recording stops, using your configured summary model."
            control={
              <Switch
                checked={isAutoSummary}
                onCheckedChange={toggleIsAutoSummary}
                aria-label="Summarize automatically when a meeting ends"
              />
            }
          />
        </SettingsGroup>
      </SettingsSection>
    </div>
  );
}
