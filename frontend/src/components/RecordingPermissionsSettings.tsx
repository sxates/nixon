'use client';

import React, { useCallback, useEffect, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { Check, Loader2, Mic, RefreshCw, Volume2 } from 'lucide-react';
import { usePermissionsModal } from '@/contexts/PermissionsModalContext';
import { usePermissionCheck } from '@/hooks/usePermissionCheck';
import { useIsLinux } from '@/hooks/usePlatform';

/**
 * "Recording permissions" control for Settings → General (spec 0038 WS7.a).
 *
 * Surfaces the current microphone + screen-recording (system audio) status and a
 * single action that opens the SAME "Enable recording" permissions modal used by
 * first-run prompting (`PermissionsModalContext`). This replaces the former
 * top-level "Permissions" sidebar nav entry — the modal itself is unchanged.
 *
 * Microphone availability is inferred from the audio-device list (the existing
 * `usePermissionCheck` pattern); screen-recording is read directly via
 * `check_screen_recording_permission_command`. Both degrade gracefully on the
 * bare dev binary (a failed read resolves to "not granted").
 */
export function RecordingPermissionsSettings() {
  const { openPermissionsModal } = usePermissionsModal();
  const { hasMicrophone, isChecking, checkPermissions } = usePermissionCheck();
  const isLinux = useIsLinux();

  const [screenRecording, setScreenRecording] = useState<boolean>(false);
  const [refreshing, setRefreshing] = useState(false);

  const refreshScreenRecording = useCallback(async () => {
    try {
      const granted = await invoke<boolean>('check_screen_recording_permission_command');
      setScreenRecording(granted);
    } catch (error) {
      // Command unavailable (bare dev binary) — treat as not granted.
      console.warn('[RecordingPermissionsSettings] screen-recording check failed:', error);
      setScreenRecording(false);
    }
  }, []);

  useEffect(() => {
    void refreshScreenRecording();
  }, [refreshScreenRecording]);

  const handleRecheck = async () => {
    setRefreshing(true);
    try {
      await Promise.all([checkPermissions(), refreshScreenRecording()]);
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
      key: 'screen',
      label: 'Screen recording',
      sub: 'Record other participants (system audio)',
      icon: <Volume2 className="h-4 w-4 text-muted-foreground" aria-hidden="true" />,
      granted: screenRecording,
    },
  ];

  return (
    <div className="bg-card rounded-lg border border-border p-6 shadow-sm">
      <div className="flex items-start justify-between gap-4">
        <div>
          <h3 className="text-lg font-semibold text-foreground mb-2">Recording permissions</h3>
          <p className="text-sm text-muted-foreground">
            Nixon needs microphone and screen-recording access to capture and transcribe
            meetings. Everything is processed locally on your Mac.
          </p>
        </div>
        <button
          type="button"
          onClick={handleRecheck}
          disabled={refreshing || isChecking}
          className="inline-flex flex-shrink-0 items-center gap-1.5 rounded-md border border-input px-2.5 py-1.5 text-xs font-medium text-muted-foreground transition-colors hover:bg-accent hover:text-foreground focus:outline-none focus-visible:ring-2 focus-visible:ring-ring disabled:opacity-60"
        >
          <RefreshCw
            className={`h-3.5 w-3.5 ${refreshing || isChecking ? 'animate-spin' : ''}`}
            aria-hidden="true"
          />
          Recheck
        </button>
      </div>

      <div className="mt-4 divide-y divide-border rounded-lg border border-border">
        {rows.map((row) => (
          <div key={row.key} className="flex items-center gap-3 px-4 py-3">
            <span className="flex h-8 w-8 flex-shrink-0 items-center justify-center rounded-lg bg-secondary">
              {row.icon}
            </span>
            <div className="min-w-0 flex-1">
              <div className="text-sm font-semibold text-foreground">{row.label}</div>
              <div className="text-xs text-muted-foreground">{row.sub}</div>
            </div>
            {row.granted ? (
              <span className="flex items-center gap-1.5 text-xs font-semibold text-chart-4">
                <Check className="h-3.5 w-3.5" aria-hidden="true" strokeWidth={2.5} />
                Allowed
              </span>
            ) : isChecking && row.key === 'mic' ? (
              <span className="flex items-center gap-1.5 text-xs text-muted-foreground">
                <Loader2 className="h-3.5 w-3.5 animate-spin" aria-hidden="true" />
                Checking…
              </span>
            ) : (
              <span className="text-xs font-semibold text-muted-foreground">Not granted</span>
            )}
          </div>
        ))}
      </div>

      <div className="mt-4">
        <button
          type="button"
          onClick={openPermissionsModal}
          className="inline-flex items-center gap-2 rounded-lg bg-brand px-3.5 py-2 text-sm font-semibold text-brand-foreground transition-colors hover:bg-brand/90 focus:outline-none focus-visible:ring-2 focus-visible:ring-ring"
        >
          Manage permissions
        </button>
      </div>
    </div>
  );
}
