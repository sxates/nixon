"use client"

import { useEffect, useState } from "react"
import { invoke } from "@tauri-apps/api/core"
import { Switch } from "./ui/switch"
import { useConfig } from "@/contexts/ConfigContext"
import { NotificationPermissionRow } from "@/components/NotificationPermissionRow"
import { SettingsGroup, SettingsRow, SettingsSection } from "@/components/ui/settings"

export function PreferenceSettings() {
  const { loadPreferences } = useConfig();

  // specs/0058 — auto-download preference; its own backend commands, its own section.
  const [autoUpdate, setAutoUpdate] = useState<boolean | null>(null);
  useEffect(() => {
    invoke<{ auto_update: boolean }>('api_get_updater_settings')
      .then((s) => setAutoUpdate(s.auto_update))
      .catch(() => setAutoUpdate(true));
  }, []);
  const onAutoUpdate = async (next: boolean) => {
    setAutoUpdate(next);
    try {
      await invoke('api_set_updater_settings', { autoUpdate: next });
    } catch (e) {
      console.error('Failed to save update setting', e);
      setAutoUpdate(!next);
    }
  };

  // Lazy load preferences on mount (only loads if not already cached)
  useEffect(() => {
    loadPreferences();
  }, [loadPreferences]);

  return (
    <div className="space-y-8">
      <SettingsSection
        title="Notifications"
        description="What Nixon can tell you when you are not looking at it."
      >
        <SettingsGroup>
          <NotificationPermissionRow />
        </SettingsGroup>
      </SettingsSection>

      <SettingsSection title="Updates" description="How Nixon gets new versions.">
        <SettingsGroup>
          <SettingsRow
            label="Download updates automatically"
            description="Checks GitHub every few hours and downloads new versions in the background. You always choose when to restart."
            control={
              <Switch
                checked={autoUpdate ?? true}
                onCheckedChange={onAutoUpdate}
                disabled={autoUpdate === null}
                aria-label="Download updates automatically"
              />
            }
          />
        </SettingsGroup>
      </SettingsSection>
    </div>
  )
}
