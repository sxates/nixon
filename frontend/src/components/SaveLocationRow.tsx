'use client';

/**
 * "Save location" settings row (specs/0061 W6). Extracted from RecordingSettings so
 * that file stays under the size ratchet (R8) — same reason ZoomMuteGateToggle is its
 * own component. This is a pure move: the "Open folder" / "Change…" behavior is
 * unchanged, just relocated so it can be unit-tested and read on its own.
 */

import type { Dispatch, SetStateAction } from 'react';
import { FolderOpen } from 'lucide-react';
import { invoke } from '@tauri-apps/api/core';
import { toast } from 'sonner';
import { Button } from '@/components/ui/button';
import { SettingsRow } from '@/components/ui/settings';
import { tildePath } from '@/lib/format-path';
import type { RecordingPreferences } from './RecordingSettings';

interface SaveLocationRowProps {
  preferences: RecordingPreferences;
  setPreferences: Dispatch<SetStateAction<RecordingPreferences>>;
  onSave?: (preferences: RecordingPreferences) => void;
}

export function SaveLocationRow({ preferences, setPreferences, onSave }: SaveLocationRowProps) {
  const handleOpenFolder = async () => {
    try {
      await invoke('open_recordings_folder');
    } catch (error) {
      console.error('Failed to open recordings folder:', error);
    }
  };

  // Change where NEW recordings are saved (specs/0061 W6). Existing meetings keep their
  // own folder_path — only new recordings land in the newly chosen folder. Persists
  // through the same set_recording_preferences path the app reads at startup
  // (recording_preferences.rs's RECORDINGS_ROOT cache is re-seeded on every save).
  const handleChangeFolder = async () => {
    let picked: string | null;
    try {
      picked = await invoke<string | null>('select_recording_folder');
    } catch (error) {
      console.error('Failed to open folder picker:', error);
      toast.error('Failed to open folder picker');
      return;
    }
    if (!picked) return; // user cancelled

    const previous = preferences;
    // `save_folder_user_chosen` marks this as a deliberate choice, which is what stops the
    // debug build's startup re-point from undoing it on the next launch.
    const newPreferences = {
      ...preferences,
      save_folder: picked,
      save_folder_user_chosen: true,
    };
    setPreferences(newPreferences);
    try {
      await invoke('set_recording_preferences', { preferences: newPreferences });
      onSave?.(newPreferences);
      toast.success('Preference saved', {
        description: 'New recordings will be saved to the selected folder.',
      });
    } catch (error) {
      console.error('Failed to save recording folder preference:', error);
      setPreferences(previous); // revert on failure
      toast.error('Failed to save preference');
    }
  };

  return (
    <SettingsRow
      label="Save location"
      description={
        // Rendered through `tildePath` so the row never puts the user's account name on
        // screen (it reached a committed screenshot once). The full path is one hover away,
        // and every path we ACT on below is the unabbreviated `save_folder`.
        <span className="break-all" title={preferences.save_folder || undefined}>
          {preferences.save_folder ? tildePath(preferences.save_folder) : 'Default folder'}
        </span>
      }
      control={
        <div className="flex items-center gap-2">
          <Button variant="outline" size="sm" onClick={handleOpenFolder}>
            <FolderOpen className="h-4 w-4" />
            Open folder
          </Button>
          <Button variant="outline" size="sm" onClick={handleChangeFolder}>
            Change…
          </Button>
        </div>
      }
    />
  );
}
