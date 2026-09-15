"use client"

import { Switch } from "./ui/switch"
import { useConfig } from "@/contexts/ConfigContext"
import {
  BetaFeatureKey,
  BETA_FEATURE_NAMES,
  BETA_FEATURE_DESCRIPTIONS
} from "@/types/betaFeatures"
import {
  SettingsGroup,
  SettingsNote,
  SettingsRow,
  SettingsSection,
} from "./ui/settings"

export function BetaSettings() {
  const { betaFeatures, toggleBetaFeature } = useConfig();

  // Define feature order for display (allows custom ordering)
  const featureOrder: BetaFeatureKey[] = ['importAndRetranscribe'];

  return (
    <SettingsSection
      title="Beta features"
      description="Still being tested. You may hit rough edges — feedback welcome."
    >
      <SettingsGroup>
        {featureOrder.map((featureKey) => (
          <SettingsRow
            key={featureKey}
            label={
              <span className="flex items-center gap-2">
                {BETA_FEATURE_NAMES[featureKey]}
                <span className="rounded-[3px] bg-brand/10 px-2 py-0.5 text-[11px] font-semibold text-brand">
                  BETA
                </span>
              </span>
            }
            description={BETA_FEATURE_DESCRIPTIONS[featureKey]}
            control={
              <Switch
                checked={betaFeatures[featureKey]}
                onCheckedChange={(checked) => toggleBetaFeature(featureKey, checked)}
                aria-label={BETA_FEATURE_NAMES[featureKey]}
              />
            }
          />
        ))}
      </SettingsGroup>

      <SettingsNote tone="muted">
        When disabled, beta features are hidden. Your existing meetings remain unaffected.
      </SettingsNote>
    </SettingsSection>
  );
}
