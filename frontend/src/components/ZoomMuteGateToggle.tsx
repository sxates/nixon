'use client';

/**
 * Zoom mute-gate settings toggle (specs/0049). Opt-in; when enabled it prompts for
 * the macOS Accessibility permission needed to read Zoom's mute state, and points the
 * user at System Settings if it isn't granted yet (the gate activates automatically
 * once it is). Self-contained so RecordingSettings stays under the size ratchet.
 */

import { useEffect, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { toast } from 'sonner';
import { Switch } from '@/components/ui/switch';
import { SettingsRow } from '@/components/ui/settings';

export default function ZoomMuteGateToggle() {
  const [enabled, setEnabled] = useState(false);

  useEffect(() => {
    void (async () => {
      try {
        setEnabled(await invoke<boolean>('api_get_zoom_mute_gate'));
      } catch (error) {
        console.error('Failed to load Zoom mute-gate preference:', error);
      }
    })();
  }, []);

  const onToggle = async (next: boolean) => {
    const previous = enabled;
    setEnabled(next);
    try {
      await invoke('api_set_zoom_mute_gate', { enabled: next });
      if (next) {
        const trusted = await invoke<boolean>('api_zoom_mute_ax_trusted', { prompt: true });
        if (!trusted) {
          toast('Accessibility permission needed', {
            description:
              "Nixon needs Accessibility access to read Zoom's mute state. Grant it in System Settings — the gate turns on automatically once you do.",
            action: {
              label: 'Open Settings',
              onClick: () => void invoke('api_open_accessibility_settings'),
            },
            duration: 12000,
          });
        } else {
          toast.success('Preference saved');
        }
      } else {
        toast.success('Preference saved');
      }
    } catch (error) {
      console.error('Failed to save Zoom mute-gate preference:', error);
      setEnabled(previous);
      toast.error('Failed to save preference');
    }
  };

  // specs/0067: a plain row, not a card of its own. It is one of the things Nixon does
  // while recording, so it belongs in that group with the others rather than sitting
  // underneath it in different clothes.
  return (
    <SettingsRow
      label="Pause mic when muted in Zoom"
      description="While recording, stop transcribing your microphone whenever you&rsquo;re muted in Zoom. Needs macOS Accessibility permission (you&rsquo;ll be prompted)."
      control={
        <Switch
          checked={enabled}
          onCheckedChange={onToggle}
          aria-label="Pause mic when muted in Zoom"
        />
      }
    />
  );
}
