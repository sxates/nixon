"use client"

import { useEffect, useState } from "react"
import { invoke } from "@tauri-apps/api/core"
import { Switch } from "./ui/switch"
import { useConfig, NotificationSettings } from "@/contexts/ConfigContext"
import { SettingsGroup, SettingsRow, SettingsSection } from "@/components/ui/settings"

export function PreferenceSettings() {
  const {
    notificationSettings,
    isLoadingPreferences,
    loadPreferences,
    updateNotificationSettings
  } = useConfig();

  const [notificationsEnabled, setNotificationsEnabled] = useState<boolean | null>(null);
  const [isInitialLoad, setIsInitialLoad] = useState(true);
  const [previousNotificationsEnabled, setPreviousNotificationsEnabled] = useState<boolean | null>(null);

  // specs/0058 — auto-download preference, independent of the notifications
  // state above (its own backend commands, its own SettingsSection).
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

  // Update notificationsEnabled when notificationSettings are loaded from global state
  useEffect(() => {
    if (notificationSettings) {
      // Notification enabled means both started and stopped notifications are enabled
      const enabled =
        notificationSettings.notification_preferences.show_recording_started &&
        notificationSettings.notification_preferences.show_recording_stopped;
      setNotificationsEnabled(enabled);
      if (isInitialLoad) {
        setPreviousNotificationsEnabled(enabled);
        setIsInitialLoad(false);
      }
    } else if (!isLoadingPreferences) {
      // If not loading and no settings, use default
      setNotificationsEnabled(true);
      if (isInitialLoad) {
        setPreviousNotificationsEnabled(true);
        setIsInitialLoad(false);
      }
    }
  }, [notificationSettings, isLoadingPreferences, isInitialLoad])

  useEffect(() => {
    // Skip update on initial load or if value hasn't actually changed
    if (isInitialLoad || notificationsEnabled === null || notificationsEnabled === previousNotificationsEnabled) return;
    if (!notificationSettings) return;

    const handleUpdateNotificationSettings = async () => {
      console.log("Updating notification settings to:", notificationsEnabled);

      try {
        // Update the notification preferences
        const updatedSettings: NotificationSettings = {
          ...notificationSettings,
          notification_preferences: {
            ...notificationSettings.notification_preferences,
            show_recording_started: notificationsEnabled,
            show_recording_stopped: notificationsEnabled,
          }
        };

        console.log("Calling updateNotificationSettings with:", updatedSettings);
        await updateNotificationSettings(updatedSettings);
        setPreviousNotificationsEnabled(notificationsEnabled);
        console.log("Successfully updated notification settings to:", notificationsEnabled);
      } catch (error) {
        console.error('Failed to update notification settings:', error);
      }
    };

    handleUpdateNotificationSettings();
  }, [notificationsEnabled, notificationSettings, isInitialLoad, previousNotificationsEnabled, updateNotificationSettings])

  // While preferences load, render the section shell with the row disabled rather
  // than a differently-styled "Loading…" box — the page keeps its shape.
  const loading =
    (isLoadingPreferences && !notificationSettings) ||
    (notificationsEnabled === null && !isLoadingPreferences);

  // Notifications only. The recordings storage location moved to the Recordings
  // tab (it duplicated the save-location row there).
  return (
    <div className="space-y-8">
      <SettingsSection
        title="Notifications"
        description="What Nixon tells you while a meeting is being recorded."
      >
        <SettingsGroup>
          <SettingsRow
            label="Meeting start and end notifications"
            description="Notify me when a recording starts and when it stops."
            control={
              <Switch
                checked={notificationsEnabled ?? false}
                onCheckedChange={setNotificationsEnabled}
                disabled={loading}
                aria-label="Meeting start and end notifications"
              />
            }
          />
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
