'use client';

import React, { useCallback, useEffect, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { Check, Loader2, Mic, RefreshCw, Volume2 } from 'lucide-react';
import { usePermissionsModal } from '@/contexts/PermissionsModalContext';
import { usePermissionCheck } from '@/hooks/usePermissionCheck';
import { useIsLinux } from '@/hooks/usePlatform';
import { Button } from '@/components/ui/button';
import { SettingsGroup, SettingsRow, SettingsSection } from '@/components/ui/settings';

/**
 * "Recording permissions" control for Settings → General (spec 0038 WS7.a).
 *
 * Surfaces the current microphone + audio-capture (system audio) status and a
 * single action that opens the SAME "Enable recording" permissions modal used by
 * first-run prompting (`PermissionsModalContext`). This replaces the former
 * top-level "Permissions" sidebar nav entry — the modal itself is unchanged.
 *
 * Microphone availability is inferred from the audio-device list (the existing
 * `usePermissionCheck` pattern); audio-capture is read directly via
 * `check_audio_capture_permission_command`. Both degrade gracefully on the
 * bare dev binary (a failed read resolves to "not granted").
 */
export function RecordingPermissionsSettings() {
  const { openPermissionsModal } = usePermissionsModal();
  const { hasMicrophone, isChecking, checkPermissions } = usePermissionCheck();
  const isLinux = useIsLinux();

  const [audioCapture, setAudioCapture] = useState<boolean>(false);
  const [refreshing, setRefreshing] = useState(false);

  const refreshAudioCapture = useCallback(async () => {
    try {
      const granted = await invoke<boolean>('check_audio_capture_permission_command');
      setAudioCapture(granted);
    } catch (error) {
      // Command unavailable (bare dev binary) — treat as not granted.
      console.warn('[RecordingPermissionsSettings] audio-capture check failed:', error);
      setAudioCapture(false);
    }
  }, []);

  useEffect(() => {
    void refreshAudioCapture();
  }, [refreshAudioCapture]);

  const handleRecheck = async () => {
    setRefreshing(true);
    try {
      await Promise.all([checkPermissions(), refreshAudioCapture()]);
    } finally {
      setRefreshing(false);
    }
  };

  // Recording permissions are a macOS concern; on Linux the app doesn't gate on them.
  if (isLinux) {
    return null;
  }

  const rows = [
    {
      key: 'mic',
      label: 'Microphone',
      sub: 'Capture your voice in the room',
      icon: <Mic className="h-4 w-4 text-muted-foreground" aria-hidden="true" />,
      granted: hasMicrophone,
    },
    {
      key: 'audio',
      label: 'Audio capture',
      sub: 'Record other participants (system audio)',
      icon: <Volume2 className="h-4 w-4 text-muted-foreground" aria-hidden="true" />,
      granted: audioCapture,
    },
  ];

  return (
    <SettingsSection
      title="Recording permissions"
      description="Nixon needs microphone and audio-capture access to capture and transcribe meetings. Everything is processed locally on your Mac."
    >
      <SettingsGroup>
        {rows.map((row) => (
          <SettingsRow
            key={row.key}
            label={
              <span className="flex items-center gap-2">
                {row.icon}
                {row.label}
              </span>
            }
            description={row.sub}
            control={
              row.granted ? (
                <span className="flex items-center gap-1.5 text-sm font-semibold text-chart-4">
                  <Check className="h-3.5 w-3.5" aria-hidden="true" strokeWidth={2.5} />
                  Allowed
                </span>
              ) : isChecking && row.key === 'mic' ? (
                <span className="flex items-center gap-1.5 text-sm text-muted-foreground">
                  <Loader2 className="h-3.5 w-3.5 animate-spin" aria-hidden="true" />
                  Checking…
                </span>
              ) : (
                <span className="text-sm font-semibold text-muted-foreground">Not granted</span>
              )
            }
          />
        ))}
        <SettingsRow
          label="Manage permissions"
          description="Open the same guided prompt Nixon shows on first run, or re-read the current status."
          control={
            <div className="flex items-center gap-2">
              <Button
                variant="outline"
                size="sm"
                onClick={handleRecheck}
                disabled={refreshing || isChecking}
              >
                <RefreshCw
                  className={`h-3.5 w-3.5 ${refreshing || isChecking ? 'animate-spin' : ''}`}
                  aria-hidden="true"
                />
                Recheck
              </Button>
              <Button variant="brand" size="sm" onClick={openPermissionsModal}>
                Manage permissions
              </Button>
            </div>
          }
        />
      </SettingsGroup>
    </SettingsSection>
  );
}
