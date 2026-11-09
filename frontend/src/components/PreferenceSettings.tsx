"use client"

import { useEffect, useState } from "react"
import { invoke } from "@tauri-apps/api/core"
import { toast } from "sonner"
import { Switch } from "./ui/switch"
import { useConfig } from "@/contexts/ConfigContext"
import { NotificationPermissionRow } from "@/components/NotificationPermissionRow"
import { SettingsGroup, SettingsRow, SettingsSection } from "@/components/ui/settings"

export function PreferenceSettings() {
  const { loadPreferences } = useConfig();

  // specs/0058 — auto-download preference; its own backend commands, its own section.
  // specs/0086 — early access lives in the same file, with its own setter.
  const [autoUpdate, setAutoUpdate] = useState<boolean | null>(null);
  const [earlyAccess, setEarlyAccess] = useState<boolean | null>(null);
  useEffect(() => {
    invoke<{ auto_update: boolean; early_access?: boolean }>('api_get_updater_settings')
      .then((s) => {
        setAutoUpdate(s.auto_update);
        setEarlyAccess(s.early_access ?? false);
      })
      .catch(() => {
        setAutoUpdate(true);
        setEarlyAccess(false);
      });
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
  const onEarlyAccess = async (next: boolean) => {
    setEarlyAccess(next);
    try {
      await invoke('api_set_early_access', { enabled: next });
    } catch (e) {
      console.error('Failed to save the early access setting', e);
      setEarlyAccess(!next);
      toast.error("Couldn't save the early access setting.");
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
            description="Checks for new versions every few hours and downloads them in the background. You always choose when to restart."
            control={
              <Switch
                checked={autoUpdate ?? true}
                onCheckedChange={onAutoUpdate}
                disabled={autoUpdate === null}
                aria-label="Download updates automatically"
              />
            }
          />
          <SettingsRow
            label="Early access updates"
            description="Get new versions before everyone else. They may have more bugs. Turning this off keeps the version you have until the next regular update."
            control={
              <Switch
                checked={earlyAccess ?? false}
                onCheckedChange={onEarlyAccess}
                disabled={earlyAccess === null}
                aria-label="Early access updates"
              />
            }
          />
        </SettingsGroup>
      </SettingsSection>
    </div>
  )
}
