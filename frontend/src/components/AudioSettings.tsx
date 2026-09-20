'use client';

/**
 * Settings → Audio (specs/0067, owner request).
 *
 * One row per input, answering both questions Nixon can be asked about it: **is it
 * allowed**, and **which device**. Those used to be two tabs apart — permission status on
 * Permissions, device pickers under Recordings → Audio devices — which is a strange split,
 * because when the microphone is not working you do not know in advance which of the two
 * is wrong. Now Microphone and System audio each carry their own status and selector on
 * one line.
 *
 * The permission itself is still granted through the same guided modal as first-run
 * (`PermissionsModalContext`); this surfaces the state and the way in, and does not invent
 * a second path to the same grant.
 */

import { useCallback, useEffect, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { Check, Loader2, Mic, RefreshCw, Volume2 } from 'lucide-react';
import { toast } from 'sonner';
import { patchRecordingPreferences } from '@/lib/recording-preferences';
import { usePermissionsModal } from '@/contexts/PermissionsModalContext';
import { usePermissionCheck } from '@/hooks/usePermissionCheck';
import { Button } from '@/components/ui/button';
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from '@/components/ui/select';
import { SettingsGroup, SettingsNote, SettingsSection } from '@/components/ui/settings';

interface AudioDevice {
  name: string;
  device_type: 'Input' | 'Output';
}

export interface AudioDeviceChoice {
  micDevice: string | null;
  systemDevice: string | null;
}

export function AudioSettings() {
  const { openPermissionsModal } = usePermissionsModal();
  const { hasMicrophone, isChecking, checkPermissions } = usePermissionCheck();

  const [audioCapture, setAudioCapture] = useState(false);
  const [devices, setDevices] = useState<AudioDevice[]>([]);
  const [refreshing, setRefreshing] = useState(false);
  const [saving, setSaving] = useState(false);
  const [selected, setSelected] = useState<AudioDeviceChoice>({ micDevice: null, systemDevice: null });

  const refresh = useCallback(async () => {
    try {
      setAudioCapture(await invoke<boolean>('check_audio_capture_permission_command'));
    } catch {
      // Command unavailable (bare dev binary) — treat as not granted.
      setAudioCapture(false);
    }
    try {
      setDevices(await invoke<AudioDevice[]>('get_audio_devices'));
    } catch {
      setDevices([]);
    }
    try {
      const prefs = await invoke<{
        preferred_mic_device: string | null;
        preferred_system_device: string | null;
      }>('get_recording_preferences');
      setSelected({
        micDevice: prefs?.preferred_mic_device ?? null,
        systemDevice: prefs?.preferred_system_device ?? null,
      });
    } catch {
      // Keep the defaults; the selects fall back to "Default …".
    }
  }, []);

  // Read-modify-write, because the Recording tab writes the same store (see
  // `patchRecordingPreferences`): a whole-object save from either side would otherwise
  // revert whatever the other had just changed.
  const onDeviceChange = async (next: AudioDeviceChoice) => {
    const previous = selected;
    setSelected(next);
    setSaving(true);
    try {
      await patchRecordingPreferences({
        preferred_mic_device: next.micDevice,
        preferred_system_device: next.systemDevice,
      });
      toast.success('Preference saved');
    } catch (error) {
      console.error('Failed to save audio device preference:', error);
      setSelected(previous);
      toast.error('Failed to save preference');
    } finally {
      setSaving(false);
    }
  };

  useEffect(() => {
    void refresh();
  }, [refresh]);

  const handleRefresh = async () => {
    setRefreshing(true);
    await Promise.all([refresh(), checkPermissions()]);
    setRefreshing(false);
  };

  const inputs = devices.filter((d) => d.device_type === 'Input');
  const outputs = devices.filter((d) => d.device_type === 'Output');

  const rows = [
    {
      key: 'mic' as const,
      icon: <Mic className="h-4 w-4 text-muted-foreground" aria-hidden />,
      label: 'Microphone',
      sub: 'Your voice in the room',
      granted: hasMicrophone,
      value: selected.micDevice ?? 'default',
      defaultLabel: 'Default microphone',
      options: inputs,
      onChange: (name: string) =>
        onDeviceChange({ ...selected, micDevice: name === 'default' ? null : name }),
    },
    {
      key: 'system' as const,
      icon: <Volume2 className="h-4 w-4 text-muted-foreground" aria-hidden />,
      label: 'System audio',
      sub: 'Everyone else on the call',
      granted: audioCapture,
      value: selected.systemDevice ?? 'default',
      defaultLabel: 'Default system audio',
      options: outputs,
      onChange: (name: string) =>
        onDeviceChange({ ...selected, systemDevice: name === 'default' ? null : name }),
    },
  ];

  return (
    <SettingsSection
      title="Audio"
      description="What Nixon is allowed to hear, and which device each side of the meeting comes from. Everything is processed on this Mac."
    >
      <SettingsGroup>
        <div className="divide-y divide-border">
          {/* Column headers, so the two columns read as one table rather than a pile of
              controls. Hidden from screen readers: each row already names itself. */}
          <div className="flex items-center gap-4 px-4 py-2" aria-hidden>
            <span className="u-section-label flex-1 text-[9px]">Input</span>
            <span className="u-section-label w-28 text-[9px]">Permission</span>
            <span className="u-section-label w-64 text-[9px]">Device</span>
          </div>

          {rows.map((row) => (
            <div key={row.key} className="flex items-center gap-4 px-4 py-3" data-testid={`audio-row-${row.key}`}>
              <div className="flex min-w-0 flex-1 items-center gap-2.5">
                {row.icon}
                <span className="min-w-0">
                  <span className="block text-sm text-foreground">{row.label}</span>
                  <span className="block text-xs text-muted-foreground">{row.sub}</span>
                </span>
              </div>

              <div className="w-28">
                {isChecking && row.key === 'mic' ? (
                  <Loader2 className="h-4 w-4 animate-spin text-muted-foreground" aria-label="Checking" />
                ) : row.granted ? (
                  <span className="inline-flex items-center gap-1.5 text-sm text-success">
                    <Check className="h-3.5 w-3.5" aria-hidden />
                    Allowed
                  </span>
                ) : (
                  <button
                    type="button"
                    onClick={openPermissionsModal}
                    className="u-section-label rounded-[3px] border border-border px-2 py-0.5 text-[9px] text-muted-foreground transition-colors hover:bg-key hover:text-foreground focus:outline-none focus-visible:ring-2 focus-visible:ring-ring"
                  >
                    Allow…
                  </button>
                )}
              </div>

              <div className="w-64">
                <Select value={row.value} onValueChange={row.onChange} disabled={saving}>
                  <SelectTrigger aria-label={`${row.label} device`}>
                    <SelectValue placeholder={row.defaultLabel} />
                  </SelectTrigger>
                  <SelectContent>
                    <SelectItem value="default">{row.defaultLabel}</SelectItem>
                    {row.options.map((device) => (
                      <SelectItem key={device.name} value={device.name}>
                        {device.name}
                      </SelectItem>
                    ))}
                  </SelectContent>
                </Select>
              </div>
            </div>
          ))}
        </div>

        <div className="flex items-center justify-between gap-3 px-4 py-3">
          <span className="text-xs text-muted-foreground">
            Devices and permissions are read when this page opens.
          </span>
          <Button variant="outline" size="sm" onClick={handleRefresh} disabled={refreshing}>
            <RefreshCw className={`mr-1.5 h-3.5 w-3.5 ${refreshing ? 'animate-spin' : ''}`} aria-hidden />
            Refresh
          </Button>
        </div>
      </SettingsGroup>

      {!hasMicrophone && !isChecking && (
        <SettingsNote tone="muted">
          Without microphone access Nixon can still record everyone else, but not you.
        </SettingsNote>
      )}
    </SettingsSection>
  );
}
